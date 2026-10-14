export function updateLabel(state:string,version:string|null,progress:number|null) {
  return state==='checking'?'正在检查更新':state==='downloading'?`正在下载更新${progress===null?'':` ${progress}%`}`:state==='verifying'?'正在验证更新包':state==='readyToInstall'?`重启并更新到 ${version}`:state==='preparing'?'正在保存并准备更新':state==='stopping'?'正在结束后台任务':state==='installing'?'正在启动安装器':state==='recovery'?'需要重启当前版本':state==='failed'?'更新失败':'检查更新';
}
