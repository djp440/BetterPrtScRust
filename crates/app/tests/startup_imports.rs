//! 常驻进程启动时不加载用不到的 DLL：设置界面（egui + glow）和常驻实例是同一个 exe，
//! `opengl32.dll` 只能延迟加载，由设置界面子进程第一次用到时才载入；`winhttp.dll` 只有开启
//! “检查更新”后才载入；录屏用的 Media Foundation（`mfplat.dll`、`mfreadwrite.dll`）和实时遮罩用的
//! DirectComposition（`dcomp.dll`）只在第一次录屏时载入。
//!
//! 直接读 exe 的 PE 导入表（普通导入 = 进程启动时由加载器载入），不需要运行程序。
#![cfg(windows)]

const RESIDENT_EXE: &str = env!("CARGO_BIN_EXE_BetterPrtSc");

#[test]
fn resident_program_loads_opengl_and_winhttp_only_on_demand() {
    assert_loaded_only_on_demand(&["opengl32.dll", "winhttp.dll"]);
}

#[test]
fn resident_program_loads_recording_dlls_only_on_demand() {
    assert_loaded_only_on_demand(&["mfplat.dll", "mfreadwrite.dll", "dcomp.dll"]);
}

/// `dlls` 不在普通导入表里（进程启动时不载入），而在延迟导入表里。
fn assert_loaded_only_on_demand(dlls: &[&str]) {
    let image = std::fs::read(RESIDENT_EXE).expect("读取 BetterPrtSc.exe");
    let pe = Pe::parse(&image);

    let loaded_at_startup = pe.imported_dlls(IMPORT_DIRECTORY, IMPORT_DESCRIPTOR_SIZE, 12);
    let loaded_on_demand = pe.imported_dlls(DELAY_IMPORT_DIRECTORY, DELAY_DESCRIPTOR_SIZE, 4);

    assert!(
        loaded_at_startup.iter().any(|dll| dll == "kernel32.dll"),
        "导入表解析有误：{loaded_at_startup:?}"
    );
    for &dll in dlls {
        assert!(
            !loaded_at_startup.iter().any(|d| d == dll),
            "启动时就会载入 {dll}：{loaded_at_startup:?}"
        );
        assert!(
            loaded_on_demand.iter().any(|d| d == dll),
            "{dll} 应当延迟加载：{loaded_on_demand:?}"
        );
    }
}

/// 数据目录里普通导入表、延迟导入表的序号，以及各自描述符的大小。
const IMPORT_DIRECTORY: usize = 1;
const IMPORT_DESCRIPTOR_SIZE: usize = 20;
const DELAY_IMPORT_DIRECTORY: usize = 13;
const DELAY_DESCRIPTOR_SIZE: usize = 32;

/// 只够读导入表的 PE32+ 解析。
struct Pe<'a> {
    image: &'a [u8],
    /// (虚拟地址, 虚拟大小, 文件偏移)
    sections: Vec<(u32, u32, u32)>,
    /// 数据目录：(RVA, 大小)
    directories: Vec<(u32, u32)>,
}

impl<'a> Pe<'a> {
    fn parse(image: &'a [u8]) -> Self {
        let pe = u32_at(image, 0x3c) as usize;
        assert_eq!(&image[pe..pe + 4], b"PE\0\0", "不是 PE 文件");
        let coff = pe + 4;
        let section_count = u16_at(image, coff + 2) as usize;
        let optional_size = u16_at(image, coff + 16) as usize;
        let optional = coff + 20;
        assert_eq!(u16_at(image, optional), 0x20b, "不是 PE32+");
        let directory_count = u32_at(image, optional + 108) as usize;
        let directories = (0..directory_count)
            .map(|i| {
                let at = optional + 112 + i * 8;
                (u32_at(image, at), u32_at(image, at + 4))
            })
            .collect();
        let sections = (0..section_count)
            .map(|i| {
                let at = optional + optional_size + i * 40;
                (
                    u32_at(image, at + 12),
                    u32_at(image, at + 8),
                    u32_at(image, at + 20),
                )
            })
            .collect();
        Self {
            image,
            sections,
            directories,
        }
    }

    fn offset(&self, rva: u32) -> usize {
        let (address, _, file) = self
            .sections
            .iter()
            .find(|(address, size, _)| rva >= *address && rva < address + size)
            .unwrap_or_else(|| panic!("RVA {rva:#x} 不在任何节里"));
        (rva - address + file) as usize
    }

    /// 某个导入目录里列出的 DLL 名（小写）。`name_field` 是描述符里 DLL 名 RVA 的位置。
    fn imported_dlls(
        &self,
        directory: usize,
        descriptor_size: usize,
        name_field: usize,
    ) -> Vec<String> {
        let (rva, _) = self.directories.get(directory).copied().unwrap_or((0, 0));
        if rva == 0 {
            return Vec::new();
        }
        let mut dlls = Vec::new();
        let mut at = self.offset(rva);
        loop {
            let name_rva = u32_at(self.image, at + name_field);
            if name_rva == 0 {
                return dlls;
            }
            let start = self.offset(name_rva);
            let end = start
                + self.image[start..]
                    .iter()
                    .position(|b| *b == 0)
                    .expect("DLL 名没有结尾");
            dlls.push(String::from_utf8_lossy(&self.image[start..end]).to_ascii_lowercase());
            at += descriptor_size;
        }
    }
}

fn u16_at(image: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([image[at], image[at + 1]])
}

fn u32_at(image: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(image[at..at + 4].try_into().unwrap())
}
