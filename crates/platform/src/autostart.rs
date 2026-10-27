use crate::PlatformError;

/// 开机自启的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AutostartMethod {
    /// 用户登录时以普通权限启动（Windows：注册表 `HKCU\...\Run` 项）。
    Normal,
    /// 用户登录时以管理员权限启动，且不弹出 UAC 提示（Windows：以最高权限运行的计划任务）。
    /// 登记和移除都需要管理员权限。
    Elevated,
}

/// 一条已登记的开机自启。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutostartEntry {
    pub method: AutostartMethod,
    /// 登记的程序是否就是当前运行的这个程序（同一路径）。
    /// 为 `false` 时它属于别处的另一份 BetterPrtSc（例如移动前的便携版、另一份安装）。
    pub this_program: bool,
}

/// 开机自启能力：每种方式最多登记一条，登记的总是当前运行的这个程序。
pub trait Autostart {
    /// 当前已登记的开机自启。
    fn entries(&self) -> Result<Vec<AutostartEntry>, PlatformError>;

    /// 用 `method` 登记开机自启，启动当前运行的这个程序；
    /// 这种方式已有登记（包括指向别处的）时覆盖它。
    fn enable(&mut self, method: AutostartMethod) -> Result<(), PlatformError>;

    /// 移除 `method` 方式的登记；本来就没有登记时什么也不做。
    fn disable(&mut self, method: AutostartMethod) -> Result<(), PlatformError>;
}

/// 本进程的权限。
pub trait Elevation {
    /// 本进程是否以管理员权限运行。
    fn is_elevated(&self) -> bool;

    /// 以管理员身份启动一个新的常驻实例来接替本进程（Windows 会弹出 UAC 提示）。
    ///
    /// 返回 `Ok` 表示新进程已经启动，本进程应尽快退出、让出单实例；
    /// 用户拒绝或启动失败时返回错误，本进程继续以当前权限运行。
    fn restart_as_administrator(&mut self) -> Result<(), PlatformError>;
}
