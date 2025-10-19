use std::io;

fn main() -> io::Result<()> {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        
        // 设置应用程序图标
        res.set_icon("assets/accurate_icon.ico");
        
        // 设置应用程序信息
        res.set("FileDescription", "SQLMap GUI Client");
        res.set("ProductName", "SQLMap GUI");
        res.set("CompanyName", "SQLMap Project");
        res.set("LegalCopyright", "Copyright © 2025 SQLMap Project");
        
        res.compile()?;
    }
    Ok(())
}