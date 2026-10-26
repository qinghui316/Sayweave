export const localAsrMessages={
 localRuntimeMissing:'本地识别组件不可用，请修复安装后重试。',localRuntimeMismatch:'本地识别组件版本不匹配，请更新应用。',
 localModelMissing:'请先下载本地模型。',localModelCorrupt:'本地模型校验失败，请重新下载。',localModelUnreadable:'无法读取本地模型，请检查文件权限。',
 localBackendUnavailable:'无法初始化计算设备，请选择仅 CPU 后重试。',localMemoryUnavailable:'可用内存或显存不足，请关闭其他程序后重试。',
 localStreamingUnsupported:'这个模型无法执行实时识别。',localConfirmationUnsupported:'这个模型无法提供可靠的流式确认。',
 localProcessUnavailable:'无法启动本地识别进程。',localPipeClosed:'本地识别进程已退出，请重新开始。',localProtocol:'本地识别组件通信异常。',
 localBufferOverflow:'本地识别处理来不及，已停止本次录音。',localPreparationTimeout:'本地模型准备超时，请重试或选择仅 CPU。',
 localPreparationFailed:'本地模型准备失败，请重新开始。',localInferenceFailed:'本地识别失败，请重新开始。',localSourceChanged:'本地模型的确认结果不一致，已停止本次识别。',
 localAudioSequence:'本地音频顺序异常，已停止本次识别。',localStaleOperation:'本地识别任务已结束，请重新开始。',localBusy:'本地识别资源仍在结算，请稍后重试。',
 localDownloadUnavailable:'无法下载本地模型，请检查网络后重试。',localDownloadTimeout:'模型下载超时，可以重试继续下载。',localDownloadRange:'下载续传校验失败，请重试。',
 localDiskFull:'磁盘空间不足，请释放空间后重试。',localFileAccess:'无法写入模型文件，请检查权限。',localConfigurationPresent:'请先删除对应的本地模型配置。',localLanguageUnsupported:'本地模型不支持这个语种。',localPlatformUnsupported:'当前平台不支持这个本地识别组件。',
} as const;
export const localAsrErrorCodes=Object.keys(localAsrMessages) as (keyof typeof localAsrMessages)[];
