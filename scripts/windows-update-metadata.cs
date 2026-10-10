// Disposable-runner-only read/query inspection; does not load the product DLL.
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
using System.Text.RegularExpressions;
using Microsoft.Win32;

namespace SayweaveMetadata
{
    public sealed class RawValue
    {
        public string scope, key, name, decoded;
        public uint root, type, byteCount;
        public ushort[] units, trailingUnits;
        public int firstNul;
        public bool terminated;
    }
    public sealed class ModuleIdentity
    {
        public string scope, path, sha256, owner;
        public long size;
    }
    public sealed class ProfileReading
    {
        public string scope, languageKey;
        public ushort language;
        public RawValue description;
    }
    public sealed class ApiReading
    {
        public ushort language;
        public string hresult, description;
        public ushort[] units;
        public uint bstrLength;
        public bool returnedNull;
    }
    public sealed class Result
    {
        public int schemaVersion = 1;
        public string kind = "read-only-tsf-metadata-characterization";
        public string classId, profileId, expectedDescription = "Sayweave Voice";
        public bool readComplete, allRawExpected, allApiExpected;
        public ModuleIdentity module;
        public ProfileReading[] registry;
        public ApiReading[] api;
    }
    public static class Characterization
    {
        static readonly Guid ClassId = new Guid("CB473CF0-9B4C-482C-BFC2-5527D2379ACA");
        static readonly Guid ProfileId = new Guid("6CD26440-E770-471E-8212-B5A565E816F5");
        static readonly Guid ProfilesClass = new Guid("33C53A50-F456-4884-B049-85FD643ECFED");
        static readonly Guid ProfilesInterface = new Guid("1F02B6C5-7842-4EE6-8A0B-9A24183A95CA");
        const string ComPath = "Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32";
        const string LanguagePath = "Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\LanguageProfile";
        const string Expected = "Sayweave Voice";
        const uint QueryAccess64 = 0x20019 | 0x100; // KEY_READ | KEY_WOW64_64KEY
        const uint MaxValueBytes = 4096;
        const long MaxModuleBytes = 32 * 1024 * 1024;

        [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        static extern int RegOpenKeyExW(IntPtr root, string path, uint options, uint access, out IntPtr key);
        [DllImport("advapi32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
        static extern int RegQueryValueExW(IntPtr key, string name, IntPtr reserved, out uint type, [Out] byte[] data, ref uint byteCount);
        [DllImport("advapi32.dll", ExactSpelling = true)]
        static extern int RegCloseKey(IntPtr key);
        [DllImport("ole32.dll", ExactSpelling = true)]
        static extern int CoInitializeEx(IntPtr reserved, uint flags);
        [DllImport("ole32.dll", ExactSpelling = true)]
        static extern void CoUninitialize();
        [DllImport("ole32.dll", ExactSpelling = true)]
        static extern int CoCreateInstance(ref Guid classId, IntPtr outer, uint context, ref Guid iid, out IntPtr instance);
        [DllImport("oleaut32.dll", ExactSpelling = true)]
        static extern uint SysStringLen(IntPtr value);
        [DllImport("oleaut32.dll", ExactSpelling = true)]
        static extern void SysFreeString(IntPtr value);
        [UnmanagedFunctionPointer(CallingConvention.StdCall)]
        delegate int DescriptionQuery(IntPtr self, ref Guid classId, ushort language, ref Guid profileId, out IntPtr description);

        public static void RequireDisposable()
        {
            if (Environment.OSVersion.Platform != PlatformID.Win32NT || IntPtr.Size != 8 ||
                Environment.GetEnvironmentVariable("GITHUB_ACTIONS") != "true" ||
                Environment.GetEnvironmentVariable("RUNNER_ENVIRONMENT") != "github-hosted" ||
                Environment.GetEnvironmentVariable("RUNNER_OS") != "Windows")
                throw new InvalidOperationException("Requires a disposable GitHub-hosted x64 Windows runner; never an owner machine.");
            using (WindowsIdentity identity = WindowsIdentity.GetCurrent())
                if (!new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))
                    throw new InvalidOperationException("Disposable runner is not elevated.");
        }
        static string Scope(uint root)
        {
            if (root == 0x80000001) return "HKEY_CURRENT_USER";
            if (root == 0x80000002) return "HKEY_LOCAL_MACHINE";
            throw new InvalidOperationException("Unknown fixed registry root.");
        }
        static RawValue RawOnce(uint root, string path, string name)
        {
            IntPtr key;
            int opened = RegOpenKeyExW(new IntPtr(unchecked((int)root)), path, 0, QueryAccess64, out key);
            if (opened == 2) return null;
            if (opened != 0) throw new InvalidOperationException("Read-only key open failed: " + opened);
            try
            {
                uint type, size = 0;
                int status = RegQueryValueExW(key, name, IntPtr.Zero, out type, null, ref size);
                if (status != 0 || type != 1 || size < 2 || size > MaxValueBytes || (size & 1) != 0)
                    throw new InvalidOperationException("Description/module value is not bounded REG_SZ: " + status);
                byte[] bytes = new byte[(int)size];
                uint actual = size, actualType;
                status = RegQueryValueExW(key, name, IntPtr.Zero, out actualType, bytes, ref actual);
                uint after = 0, afterType;
                int checkedStatus = RegQueryValueExW(key, name, IntPtr.Zero, out afterType, null, ref after);
                if (status != 0 || checkedStatus != 0 || actual != size || after != size || actualType != type || afterType != type)
                    throw new InvalidOperationException("Registry value identity changed while reading.");
                ushort[] units = new ushort[bytes.Length / 2];
                int nul = -1;
                for (int n = 0; n < units.Length; n++)
                {
                    units[n] = (ushort)(bytes[n * 2] | bytes[n * 2 + 1] << 8);
                    if (nul < 0 && units[n] == 0) nul = n;
                }
                int count = nul < 0 ? units.Length : nul;
                ushort[] trailing = new ushort[nul < 0 ? 0 : units.Length - nul - 1];
                if (trailing.Length != 0) Array.Copy(units, nul + 1, trailing, 0, trailing.Length);
                return new RawValue { scope = Scope(root), root = root, key = path, name = name ?? "", type = type,
                    byteCount = actual, units = units, firstNul = nul, terminated = nul >= 0, trailingUnits = trailing,
                    decoded = Encoding.Unicode.GetString(bytes, 0, count * 2) };
            }
            finally { RegCloseKey(key); }
        }
        static bool Same(RawValue a, RawValue b)
        {
            if (a == null || b == null) return a == b;
            if (a.type != b.type || a.byteCount != b.byteCount || a.units.Length != b.units.Length) return false;
            for (int n = 0; n < a.units.Length; n++) if (a.units[n] != b.units[n]) return false;
            return true;
        }
        static RawValue Raw(uint root, string path, string name)
        {
            RawValue first = RawOnce(root, path, name), second = RawOnce(root, path, name);
            if (!Same(first, second)) throw new InvalidOperationException("Registry value changed between read-only snapshots.");
            return first;
        }
        static string Canonical(string path)
        {
            if (path.StartsWith("\\\\?\\", StringComparison.Ordinal) && !path.StartsWith("\\\\?\\UNC\\", StringComparison.OrdinalIgnoreCase)) path = path.Substring(4);
            if (!Path.IsPathRooted(path)) throw new InvalidOperationException("Module path is not absolute.");
            return Path.GetFullPath(path);
        }
        static ModuleIdentity Module(string expectedPath, string expectedHash)
        {
            ModuleIdentity found = null;
            foreach (uint root in new uint[] { 0x80000001, 0x80000002 })
            {
                RawValue path = Raw(root, ComPath, null);
                if (path == null) continue;
                RawValue owner = Raw(root, ComPath, "SayweaveOwner");
                if (found != null || owner == null || owner.decoded != "SayweaveStageA-v1" ||
                    !String.Equals(Canonical(path.decoded), expectedPath, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidOperationException("Installed owned module is ambiguous or differs from the expected path.");
                long size; string hash;
                using (FileStream file = new FileStream(path.decoded, FileMode.Open, FileAccess.Read, FileShare.Read))
                {
                    size = file.Length;
                    if (size < 1 || size > MaxModuleBytes) throw new InvalidOperationException("Installed module exceeds bound.");
                    using (SHA256 sha = SHA256.Create()) hash = BitConverter.ToString(sha.ComputeHash(file)).Replace("-", "").ToLowerInvariant();
                }
                if (hash != expectedHash) throw new InvalidOperationException("Installed module hash differs from verified ordinary DLL.");
                found = new ModuleIdentity { scope = Scope(root), path = path.decoded, sha256 = hash, size = size, owner = owner.decoded };
            }
            if (found == null) throw new InvalidOperationException("No verified owned module is installed.");
            return found;
        }
        static List<ProfileReading> Profiles()
        {
            List<ProfileReading> result = new List<ProfileReading>();
            foreach (uint root in new uint[] { 0x80000001, 0x80000002 })
            using (RegistryKey hive = RegistryKey.OpenBaseKey(root == 0x80000001 ? RegistryHive.CurrentUser : RegistryHive.LocalMachine, RegistryView.Registry64))
            using (RegistryKey languages = hive.OpenSubKey(LanguagePath, false))
            {
                if (languages == null) continue;
                if (languages.SubKeyCount > 32) throw new InvalidOperationException("Owned language inventory exceeds bound.");
                string[] names = languages.GetSubKeyNames();
                if (names.Length > 32) throw new InvalidOperationException("Owned language inventory changed or exceeded bound.");
                Array.Sort(names, StringComparer.OrdinalIgnoreCase);
                foreach (string name in names)
                {
                    if (!Regex.IsMatch(name, "^0x[0-9a-fA-F]{8}$")) throw new InvalidOperationException("Unexpected owned language key.");
                    uint language = Convert.ToUInt32(name.Substring(2), 16);
                    if (language == 0 || language > ushort.MaxValue) throw new InvalidOperationException("Owned language does not fit LANGID.");
                    string key = LanguagePath + "\\" + name + "\\" + ProfileId.ToString("B").ToUpperInvariant();
                    RawValue value = Raw(root, key, "Description");
                    if (value != null) result.Add(new ProfileReading { scope = Scope(root), languageKey = name, language = (ushort)language, description = value });
                }
            }
            if (result.Count < 1 || result.Count > 64) throw new InvalidOperationException("Owned profile Description inventory is empty or exceeds bound.");
            return result;
        }
        static ApiReading Query(IntPtr profiles, DescriptionQuery call, ushort language)
        {
            IntPtr value = IntPtr.Zero;
            Guid clsid = ClassId, profile = ProfileId;
            int hr = call(profiles, ref clsid, language, ref profile, out value);
            try
            {
                ApiReading result = new ApiReading { language = language, hresult = "0x" + unchecked((uint)hr).ToString("X8"), returnedNull = value == IntPtr.Zero };
                if (hr < 0) return result;
                uint length = value == IntPtr.Zero ? 0 : SysStringLen(value);
                if (length > MaxValueBytes / 2) throw new InvalidOperationException("TSF description BSTR exceeds bound.");
                short[] signed = new short[(int)length];
                if (signed.Length != 0) Marshal.Copy(value, signed, 0, signed.Length);
                result.units = new ushort[signed.Length];
                char[] text = new char[signed.Length];
                for (int n = 0; n < signed.Length; n++) { result.units[n] = unchecked((ushort)signed[n]); text[n] = (char)result.units[n]; }
                result.bstrLength = length; result.description = new string(text); return result;
            }
            finally { if (value != IntPtr.Zero) SysFreeString(value); }
        }
        public static Result Read(string expectedModulePath, string expectedHash)
        {
            RequireDisposable();
            if (!Regex.IsMatch(expectedHash ?? "", "^[a-f0-9]{64}$")) throw new InvalidOperationException("Exact ordinary DLL hash required.");
            string expectedPath = Canonical(expectedModulePath);
            ModuleIdentity before = Module(expectedPath, expectedHash);
            List<ProfileReading> readings = Profiles();
            int init = CoInitializeEx(IntPtr.Zero, 2); // COINIT_APARTMENTTHREADED
            if (init < 0) Marshal.ThrowExceptionForHR(init);
            IntPtr profiles = IntPtr.Zero;
            List<ApiReading> api = new List<ApiReading>();
            try
            {
                Guid clsid = ProfilesClass, iid = ProfilesInterface;
                int hr = CoCreateInstance(ref clsid, IntPtr.Zero, 1, ref iid, out profiles); // CLSCTX_INPROC_SERVER; Windows profiles interface only
                if (hr < 0) Marshal.ThrowExceptionForHR(hr);
                if (profiles == IntPtr.Zero) throw new InvalidOperationException("Profiles interface is null.");
                // Pinned windows0.62.2 ITfInputProcessorProfiles_Vtbl: IUnknown3 + nine preceding methods => slot12.
                IntPtr vtable = Marshal.ReadIntPtr(profiles);
                DescriptionQuery query = (DescriptionQuery)Marshal.GetDelegateForFunctionPointer(Marshal.ReadIntPtr(vtable, 12 * IntPtr.Size), typeof(DescriptionQuery));
                HashSet<ushort> languages = new HashSet<ushort>();
                foreach (ProfileReading reading in readings) if (languages.Add(reading.language)) api.Add(Query(profiles, query, reading.language));
            }
            finally { if (profiles != IntPtr.Zero) Marshal.Release(profiles); CoUninitialize(); }
            List<ProfileReading> after = Profiles();
            if (after.Count != readings.Count) throw new InvalidOperationException("Owned profile inventory changed during COM query.");
            for (int n = 0; n < readings.Count; n++)
                if (readings[n].scope != after[n].scope || readings[n].languageKey != after[n].languageKey || !Same(readings[n].description, after[n].description))
                    throw new InvalidOperationException("Owned Description changed during COM query.");
            ModuleIdentity finalModule = Module(expectedPath, expectedHash);
            if (before.scope != finalModule.scope || before.path != finalModule.path || before.size != finalModule.size)
                throw new InvalidOperationException("Installed module identity changed during COM query.");
            bool rawExpected = true, apiExpected = true;
            foreach (ProfileReading reading in readings)
                rawExpected &= reading.description.decoded == Expected && reading.description.firstNul == Expected.Length && reading.description.trailingUnits.Length == 0;
            foreach (ApiReading reading in api) apiExpected &= reading.hresult == "0x00000000" && reading.description == Expected && reading.bstrLength == Expected.Length;
            return new Result { classId = ClassId.ToString("B"), profileId = ProfileId.ToString("B"), module = before,
                registry = readings.ToArray(), api = api.ToArray(), readComplete = true, allRawExpected = rawExpected, allApiExpected = apiExpected };
        }
    }
}
