function Set-SayweaveInstallerTestOwner {
  if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Installer test token setup requires a disposable GitHub-hosted Windows runner.'
  }
  if (-not ('SayweaveInstallerTestToken' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;

public static class SayweaveInstallerTestToken {
    [DllImport("kernel32.dll")] private static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("advapi32.dll", SetLastError = true)] private static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)] private static extern bool GetTokenInformation(IntPtr token, int kind, IntPtr data, int size, out int needed);
    [DllImport("advapi32.dll", SetLastError = true)] private static extern bool SetTokenInformation(IntPtr token, int kind, ref IntPtr data, int size);

    public static bool SetCurrentUserOwner() {
        const uint QueryAndAdjustDefault = 0x0088;
        const int TokenUser = 1;
        const int TokenOwner = 4;
        IntPtr token;
        if (!OpenProcessToken(GetCurrentProcess(), QueryAndAdjustDefault, out token)) throw new Win32Exception();
        IntPtr user = IntPtr.Zero;
        IntPtr owner = IntPtr.Zero;
        try {
            user = Read(token, TokenUser);
            owner = Read(token, TokenOwner);
            IntPtr sid = Marshal.ReadIntPtr(user);
            var expected = new SecurityIdentifier(sid);
            bool changed = !expected.Equals(new SecurityIdentifier(Marshal.ReadIntPtr(owner)));
            if (changed && !SetTokenInformation(token, TokenOwner, ref sid, IntPtr.Size)) throw new Win32Exception();
            return changed;
        } finally {
            if (owner != IntPtr.Zero) Marshal.FreeHGlobal(owner);
            if (user != IntPtr.Zero) Marshal.FreeHGlobal(user);
            CloseHandle(token);
        }
    }

    private static IntPtr Read(IntPtr token, int kind) {
        int size;
        GetTokenInformation(token, kind, IntPtr.Zero, 0, out size);
        if (size <= 0 || size > 4096) throw new Win32Exception();
        IntPtr data = Marshal.AllocHGlobal(size);
        if (GetTokenInformation(token, kind, data, size, out size)) return data;
        int error = Marshal.GetLastWin32Error();
        Marshal.FreeHGlobal(data);
        throw new Win32Exception(error);
    }
}
'@
  }
  $changed = [SayweaveInstallerTestToken]::SetCurrentUserOwner()
  Write-Output "Installer test token owner set to the current Windows user (changed=$changed)."
}
