#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxus::prelude::*;
use dioxus_desktop::{Config, tao::window::WindowBuilder};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, RwLock, Mutex};
use std::thread;
use std::time::Duration;
use tokio::time;


// 获取嵌入的sqlmap文件内容的函数
fn get_embedded_sqlmap() -> Result<String, Box<dyn std::error::Error>> {
    // 尝试从嵌入资源读取sqlmap.py
    let sqlmap_content = include_str!("sqlmap/sqlmap.py");
    Ok(sqlmap_content.to_string())
}

// 创建临时sqlmap文件的函数
fn create_temp_sqlmap_file() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // 获取嵌入的sqlmap内容
    let sqlmap_content = get_embedded_sqlmap()?;
    
    // 创建临时目录
    let temp_dir = std::env::temp_dir().join("sqlmap_gui");
    std::fs::create_dir_all(&temp_dir)?;
    
    // 创建临时文件路径
    let temp_file = temp_dir.join("sqlmap.py");
    
    // 写入sqlmap内容到临时文件
    std::fs::write(&temp_file, sqlmap_content)?;
    
    // 复制其他必要的sqlmap文件到临时目录
    copy_sqlmap_resources(&temp_dir)?;
    
    Ok(temp_file)
}

// 复制sqlmap资源文件的函数
fn copy_sqlmap_resources(temp_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // 获取当前工作目录（项目根目录）
    let current_dir = std::env::current_dir()?;
    
    // 查找sqlmap目录 - 尝试多个可能的路径
    let possible_paths = [
        current_dir.join("src").join("sqlmap"),  // 开发环境路径
        current_dir.join("sqlmap"),               // 发布环境路径
        current_dir.join("..").join("src").join("sqlmap"), // 调试环境路径
    ];
    
    let mut sqlmap_source_dir = None;
    
    for path in &possible_paths {
        if path.exists() {
            sqlmap_source_dir = Some(path);
            break;
        }
    }
    
    if let Some(source_dir) = sqlmap_source_dir {
        // 如果sqlmap目录存在，直接复制整个目录
        copy_dir_all(source_dir, temp_dir)?;
        println!("成功复制完整SQLmap目录到临时目录: {:?}", source_dir);
    } else {
        // 如果sqlmap目录不存在，使用嵌入的资源
        println!("警告: 未找到SQLmap目录，使用嵌入的资源文件");
        
        // 复制关键文件
        let sqlmap_conf_content = include_str!("sqlmap/sqlmap.conf");
        std::fs::write(temp_dir.join("sqlmap.conf"), sqlmap_conf_content)?;
        
        let sqlmapapi_content = include_str!("sqlmap/sqlmapapi.py");
        std::fs::write(temp_dir.join("sqlmapapi.py"), sqlmapapi_content)?;
        
        let sqlmapapi_yaml_content = include_str!("sqlmap/sqlmapapi.yaml");
        std::fs::write(temp_dir.join("sqlmapapi.yaml"), sqlmapapi_yaml_content)?;
    }
    
    Ok(())
}

// 递归复制目录的函数
fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dst)?;
    
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    
    Ok(())
}


// SQLmap配置结构体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SqlmapConfig {
    // SQLmap路径设置
    sqlmap_path: String,
    
    // 目标设置
    target_url: String,
    
    // 扫描设置
    delay: u32,
    timeout: u32,
    verbosity: u32,
    level: u32,
    risk: u32,
    
    // 枚举选项
    current_db: bool,
    current_user: bool,
    current_user_dba: bool,
    enumerate_dbs: bool,
    enumerate_tables: bool,
    enumerate_columns: bool,
    enumerate_fields: bool,
    dump_all: bool,
    os_shell: bool,
    sql_shell: bool,
    purge_cache: bool,
    
    // 新增字段：自定义参数和数据库类型
    custom_parameters: String,
    dbms_type: String,
    
    // 新增字段：指定库名、表名、列名
    database_name: String,
    table_name: String,
    column_name: String,
    
    // 新增字段：绕过模板(tamper)选择
    selected_tampers: Vec<String>,
}

impl Default for SqlmapConfig {
    fn default() -> Self {
        Self {
            sqlmap_path: String::new(),
            target_url: String::new(),
            delay: 0,
            timeout: 0,
            verbosity: 0,
            level: 3, // 默认扫描级别为3
            risk: 2,   // 默认风险级别为2
            
            // 枚举选项默认值
            current_db: false,
            current_user: false,
            current_user_dba: false,
            enumerate_dbs: false,
            enumerate_tables: false,
            enumerate_columns: false,
            enumerate_fields: false,
            dump_all: false,
            os_shell: false,
            sql_shell: false,
            purge_cache: false,
            
            // 新增字段默认值
            custom_parameters: String::new(),
            dbms_type: String::from("auto"), // 默认自动检测
            
            // 新增字段默认值
            database_name: String::new(),
            table_name: String::new(),
            column_name: String::new(),
            
            // 新增字段默认值
            selected_tampers: Vec::new(), // 默认不选择任何绕过模板
        }
    }
}

// 应用全局状态 - 线程安全
struct AppState {
    config: RwLock<SqlmapConfig>,
    is_running: RwLock<bool>,
    output: RwLock<String>,
    current_process: Mutex<Option<u32>>, // 存储进程ID
    interaction_input: RwLock<String>,
    interaction_pending: RwLock<bool>,
    child_stdin: Mutex<Option<std::process::ChildStdin>>,
    theme: RwLock<String>, // 主题状态："light" 或 "dark"
}

fn main() {
    // 初始化默认配置，自动设置项目内的sqlmap.py路径
    let mut default_config = SqlmapConfig::default();
    default_config.sqlmap_path = "src/sqlmap/sqlmap.py".to_string();
    
    // 初始化应用状态
    let state = Arc::new(AppState {
        config: RwLock::new(default_config),
        is_running: RwLock::new(false),
        output: RwLock::new(String::new()),
        current_process: Mutex::new(None),
        interaction_input: RwLock::new(String::new()),
        interaction_pending: RwLock::new(false),
        child_stdin: Mutex::new(None),
        theme: RwLock::new("light".to_string()), // 默认主题为白天模式
    });
    
    // 创建窗口构建器 - 无边框圆角窗口
    let window_builder = WindowBuilder::new()
        .with_title("SQLmap GUI")
        .with_resizable(true)
        .with_inner_size(dioxus_desktop::tao::dpi::LogicalSize::new(1400.0, 800.0))
        .with_decorations(false) // 禁用原生窗口装饰
        .with_transparent(true); // 启用透明背景以支持自定义标题栏
    
    // 配置窗口设置
    let config = Config::new()
        .with_window(window_builder)
        // 内联CSS样式
        .with_custom_head(format!("<style>\n{}\n</style>", include_str!("../src/style.css")));
    
    // 启动应用
    dioxus_desktop::launch_with_props(App, state, config);
    
    // 注意：由于Dioxus桌面版API的限制，我们无法在窗口创建后立即设置DWM圆角
    // 但我们已经通过CSS为主容器设置了圆角效果，这将在无边框窗口中提供视觉上的圆角
}

// 主应用组件
fn App(cx: Scope<Arc<AppState>>) -> Element {
    // 活动标签页状态
    let active_tab = use_state(&cx, || "config");
    
    // 本地UI状态
    let config = use_state(&cx, || SqlmapConfig::default());
    let is_running = use_state(&cx, || false);
    let output = use_state(&cx, || String::new());
    let theme = use_state(&cx, || "light".to_string());
    
    // Tamper覆盖页面状态
    let show_tamper_overlay = use_state(&cx, || false);
    let tamper_search_query = use_state(&cx, || String::new());
    let is_closing = use_state(&cx, || false);
    
    // 状态引用 - 为每个闭包创建独立的克隆
    let state = cx.props.clone();
    
    // 为每个需要使用state的闭包创建独立的克隆
    let state_for_left_refresh = state.clone();
    let state_for_config_manage_refresh = state.clone();
    
    // 自动刷新UI - 使用use_future实现定期刷新
    use_future(&cx, (), |_| {
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        let theme = theme.clone();
        
        async move {
            let mut last_output = String::new();
            loop {
                // 检查输出是否有变化
                if let Ok(global_output) = state.output.read() {
                    if *global_output != last_output {
                        last_output = global_output.clone();
                        output.set(last_output.clone());
                    }
                }
                
                // 检查运行状态是否有变化
                if let Ok(global_running) = state.is_running.read() {
                    if *global_running != *is_running.get() {
                        is_running.set(*global_running);
                    }
                }
                
                // 检查配置是否有变化
                if let Ok(global_config) = state.config.read() {
                    if *global_config != *config.get() {
                        config.set(global_config.clone());
                    }
                }
                
                // 检查主题是否有变化
                if let Ok(global_theme) = state.theme.read() {
                    if *global_theme != *theme.get() {
                        theme.set(global_theme.clone());
                    }
                }
                
                // 每100ms检查一次
                time::sleep(time::Duration::from_millis(100)).await;
            }
        }
    });
    
    // 初始化时从全局状态加载数据
    use_effect(&cx, (), {
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        let theme = theme.clone();
        
        move |_| {
            // 只在UI线程中初始化，不使用线程
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
            
            if let Ok(global_theme) = state.theme.read() {
                theme.set(global_theme.clone());
            }
            
            // 返回空future
            async move {}
        }
    });
    
    // 监听输出变化并自动滚动到底部
    // 使用use_effect监听输出变化
    use_effect(&cx, &*output.get(), {
        let output = output.clone();
        
        move |_| {
            // 当输出变化时，自动滚动到输出容器底部
            // 使用Dioxus提供的eval方法来执行JavaScript
            
            async move {
                // 等待一小段时间确保DOM已更新
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                
                // 执行JavaScript代码来滚动到底部
                // 使用更安全的方法，避免生命周期问题
                let _ = output.get();
                
                // 注意：由于Dioxus桌面环境的限制，我们使用CSS样式来实现自动滚动
                // 通过CSS的scroll-snap和overflow-anchor属性来实现自动滚动效果
            }
        }
    });
    
    // 刷新UI的逻辑现在直接嵌入到按钮的onclick事件中
    
    // 为每个字符串配置字段创建独立的更新函数
    let update_target_url = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            let mut new_config = config.get().clone();
            new_config.target_url = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    

    

    
    // 为每个数字配置字段创建独立的更新函数
    let update_delay = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            if let Ok(num) = value.parse::<u32>() {
                let mut new_config = config.get().clone();
                new_config.delay = num;
                
                // 更新本地状态
                config.set(new_config.clone());
                
                // 更新全局状态
                if let Ok(mut global_config) = state.config.write() {
                    *global_config = new_config;
                }
            }
        }
    };
    

    
    let update_verbosity = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            if let Ok(num) = value.parse::<u32>() {
                let mut new_config = config.get().clone();
                new_config.verbosity = num;
                
                // 更新本地状态
                config.set(new_config.clone());
                
                // 更新全局状态
                if let Ok(mut global_config) = state.config.write() {
                    *global_config = new_config;
                }
            }
        }
    };
    
    let update_level = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            if let Ok(num) = value.parse::<u32>() {
                let mut new_config = config.get().clone();
                new_config.level = num;
                
                // 更新本地状态
                config.set(new_config.clone());
                
                // 更新全局状态
                if let Ok(mut global_config) = state.config.write() {
                    *global_config = new_config;
                }
            }
        }
    };
    
    let update_risk = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            if let Ok(num) = value.parse::<u32>() {
                let mut new_config = config.get().clone();
                new_config.risk = num;
                
                // 更新本地状态
                config.set(new_config.clone());
                
                // 更新全局状态
                if let Ok(mut global_config) = state.config.write() {
                    *global_config = new_config;
                }
            }
        }
    };

    // 为每个布尔配置字段创建独立的更新函数
    let update_current_db = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.current_db = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_current_user = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.current_user = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_current_user_dba = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.current_user_dba = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_enumerate_dbs = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.enumerate_dbs = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_enumerate_tables = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.enumerate_tables = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_enumerate_columns = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.enumerate_columns = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_enumerate_fields = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.enumerate_fields = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_dump_all = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.dump_all = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_os_shell = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.os_shell = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };

    let update_sql_shell = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.sql_shell = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新清除缓存选项
    let update_purge_cache = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: bool| {
            let mut new_config = config.get().clone();
            new_config.purge_cache = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新自定义参数
    let update_custom_parameters = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            let mut new_config = config.get().clone();
            new_config.custom_parameters = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新数据库类型
    let update_dbms_type = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            let mut new_config = config.get().clone();
            new_config.dbms_type = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新库名
    let update_database_name = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            let mut new_config = config.get().clone();
            new_config.database_name = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新表名
    let update_table_name = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            let mut new_config = config.get().clone();
            new_config.table_name = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新列名
    let update_column_name = {
        let state = state.clone();
        let config = config.clone();
        
        move |value: String| {
            let mut new_config = config.get().clone();
            new_config.column_name = value;
            
            // 更新本地状态
            config.set(new_config.clone());
            
            // 更新全局状态
            if let Ok(mut global_config) = state.config.write() {
                *global_config = new_config;
            }
        }
    };
    
    // 更新选择的绕过模板
    // 创建一个函数来处理tamper选择更新
    fn update_selected_tampers_fn(
        state: std::sync::Arc<AppState>,
        config: &dioxus::prelude::UseState<SqlmapConfig>,
        value: Vec<String>
    ) {
        let mut new_config = config.get().clone();
        new_config.selected_tampers = value;
        
        // 更新本地状态
        config.set(new_config.clone());
        
        // 更新全局状态
        if let Ok(mut global_config) = state.config.write() {
            *global_config = new_config;
        }
    }
    

    

    
    // 保存配置
    let save_config = { 
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        
        move |_: dioxus::prelude::MouseEvent| {
            // 在UI线程中克隆状态用于工作线程
            let state_clone = state.clone();
            
            // 在新线程中执行保存操作
            thread::spawn(move || {
                // 读取全局配置
                if let Ok(global_config) = state_clone.config.read() {
                    // 克隆配置用于序列化
                    let config_to_save = global_config.clone();
                    
                    // 序列化并保存
                    if let Ok(json) = serde_json::to_string_pretty(&config_to_save) {
                        let save_result = std::fs::write("sqlmap_config.json", json);
                        
                        // 更新全局输出
                        if let Ok(mut global_output) = state_clone.output.write() {
                            match save_result {
                                Ok(_) => *global_output = "配置保存成功".to_string(),
                                Err(e) => *global_output = format!("保存配置失败: {}", e),
                            }
                        }
                    } else {
                        // 更新全局输出
                        if let Ok(mut global_output) = state_clone.output.write() {
                            *global_output = "序列化配置失败".to_string();
                        }
                    }
                }
            });
            
            // 立即刷新UI以显示最新状态
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
        }
    };
    
    // 加载配置
    let load_config = { 
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        
        move |_: dioxus::prelude::MouseEvent| {
            // 在UI线程中克隆状态用于工作线程
            let state_clone = state.clone();
            
            // 在新线程中执行加载操作
            thread::spawn(move || {
                let mut load_message = String::new();
                let mut loaded_config = SqlmapConfig::default();
                let mut load_success = false;
                
                // 尝试打开并读取配置文件
                if let Ok(mut file) = File::open("sqlmap_config.json") {
                    let mut json = String::new();
                    if file.read_to_string(&mut json).is_ok() {
                        // 反序列化配置
                        if let Ok(new_config) = serde_json::from_str::<SqlmapConfig>(&json) {
                            loaded_config = new_config;
                            
                            // 使用嵌入的sqlmap，清空sqlmap_path字段
                            loaded_config.sqlmap_path = String::new();
                            
                            load_message = "配置加载成功".to_string();
                            load_success = true;
                        } else {
                            load_message = "解析配置失败".to_string();
                        }
                    } else {
                        load_message = "读取配置文件失败".to_string();
                    }
                } else {
                    load_message = "打开配置文件失败".to_string();
                }
                
                // 更新全局输出
                if let Ok(mut global_output) = state_clone.output.write() {
                    *global_output = load_message;
                }
                
                // 如果加载成功，更新全局配置
                if load_success {
                    if let Ok(mut global_config) = state_clone.config.write() {
                        *global_config = loaded_config;
                    }
                }
            });
            
            // 立即刷新UI以显示最新状态
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
        }
    };
    
    // 运行SQLmap
    let run_sqlmap = { 
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        
        move |_| {
            // 在UI线程中克隆状态用于工作线程
            let state_clone = state.clone();
            
            // 更新全局状态
            if let Ok(mut global_running) = state_clone.is_running.write() {
                *global_running = true;
            }
            
            if let Ok(mut global_output) = state_clone.output.write() {
                *global_output = "正在运行SQLmap...\n".to_string();
            }
            
            // 立即刷新UI
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
            
            // 在新线程中执行运行操作
            thread::spawn(move || {
                // 读取全局配置
                let config = if let Ok(global_config) = state_clone.config.read() {
                    global_config.clone()
                } else {
                    if let Ok(mut global_output) = state_clone.output.write() {
                        global_output.push_str("无法读取配置");
                    }
                    if let Ok(mut global_running) = state_clone.is_running.write() {
                        *global_running = false;
                    }
                    return;
                };
                
                // 使用嵌入的sqlmap文件，创建临时文件
                let sqlmap_path = match create_temp_sqlmap_file() {
                    Ok(path) => path.to_string_lossy().to_string(),
                    Err(e) => {
                        if let Ok(mut global_output) = state_clone.output.write() {
                            global_output.push_str(&format!("创建临时sqlmap文件失败: {}\n", e));
                        }
                        if let Ok(mut global_running) = state_clone.is_running.write() {
                            *global_running = false;
                        }
                        return;
                    }
                };
                
                // 检查目标是否设置
                if config.target_url.is_empty() {
                    if let Ok(mut global_output) = state_clone.output.write() {
                        global_output.push_str("请设置扫描目标");
                    }
                    if let Ok(mut global_running) = state_clone.is_running.write() {
                        *global_running = false;
                    }
                    return;
                }
                
                // 构建命令参数
                let mut args = Vec::new();
                
                // 添加目标参数
                if !config.target_url.is_empty() {
                    // 检查输入的是URL还是数据包
                    if config.target_url.trim().starts_with("http") {
                        // 如果是URL，使用-u参数
                        args.push("-u".to_string());
                        args.push(config.target_url.clone());
                    } else {
                        // 如果是数据包，使用-r参数并保存到固定临时文件
                        let temp_file = "temp_request.txt";
                        if let Err(e) = std::fs::write(&temp_file, &config.target_url) {
                            if let Ok(mut global_output) = state_clone.output.write() {
                                global_output.push_str(&format!("创建临时文件失败: {}", e));
                            }
                            return;
                        }
                        args.push("-r".to_string());
                        args.push(temp_file.to_string());
                    }
                }
                

                
                // 添加扫描设置
                if config.delay > 0 {
                    args.push("--delay".to_string());
                    args.push(config.delay.to_string());
                }
                if config.verbosity > 0 {
                    args.push("--verbosity".to_string());
                    args.push(config.verbosity.to_string());
                }
                if config.level > 0 {
                    args.push("--level".to_string());
                    args.push(config.level.to_string());
                }
                if config.risk > 0 {
                    args.push("--risk".to_string());
                    args.push(config.risk.to_string());
                }
                
                // 添加枚举选项与sqlmap参数的对应关系
                if config.current_db {
                    args.push("--current-db".to_string());
                }
                if config.current_user {
                    args.push("--current-user".to_string());
                }
                if config.current_user_dba {
                    args.push("--is-dba".to_string());
                }
                if config.enumerate_dbs {
                    args.push("--dbs".to_string());
                }
                if config.enumerate_tables {
                    args.push("--tables".to_string());
                }
                if config.enumerate_columns {
                    args.push("--columns".to_string());
                }
                if config.enumerate_fields {
                    args.push("--dump".to_string());
                }
                if config.dump_all {
                    args.push("--dump-all".to_string());
                }
                if config.os_shell {
                    args.push("--os-shell".to_string());
                }
                if config.sql_shell {
                    args.push("--sql-shell".to_string());
                }
                if config.purge_cache {
                    args.push("--purge".to_string());
                }
                
                // 添加库名、表名、列名参数
                if !config.database_name.is_empty() {
                    args.push("-D".to_string());
                    args.push(config.database_name.trim().to_string());
                }
                if !config.table_name.is_empty() {
                    args.push("-T".to_string());
                    args.push(config.table_name.trim().to_string());
                }
                if !config.column_name.is_empty() {
                    args.push("-C".to_string());
                    args.push(config.column_name.trim().to_string());
                }
                
                // 添加其他常用参数
                // args.push("--batch".to_string()); // 非交互模式
                // args.push("--non-interactive".to_string()); // 完全非交互模式
                args.push("--ignore-stdin".to_string()); // 忽略STDIN输入检测，确保交互模式正常工作
                
                // 默认添加--random-agent参数
                args.push("--random-agent".to_string());
                
                // 添加数据库类型参数
                if config.dbms_type != "auto" {
                    args.push(format!("--dbms={}", config.dbms_type.trim()));
                }
                
                // 添加绕过模板参数
                if !config.selected_tampers.is_empty() {
                    let tampers_str = config.selected_tampers.join(",");
                    args.push(format!("--tamper={}", tampers_str));
                }
                
                // 添加自定义参数
                if !config.custom_parameters.is_empty() {
                    // 分割自定义参数并添加到args中
                    let custom_args: Vec<&str> = config.custom_parameters.split_whitespace().collect();
                    for arg in custom_args {
                        if !arg.trim().is_empty() {
                            args.push(arg.trim().to_string());
                        }
                    }
                }
                
                // 执行SQLmap命令
                use std::process::Command;
                
                // 检查操作系统并决定如何执行python脚本
                let python_cmd = if cfg!(target_os = "windows") {
                    "python"
                } else {
                    "python3"
                };
                
                if let Ok(mut global_output) = state_clone.output.write() {
                    global_output.push_str(&format!("执行命令: {} {} {}\n", python_cmd, sqlmap_path, args.join(" ")));
                }
                
                // 执行命令 - 使用pythonw避免控制台缓冲问题
                let pythonw_cmd = if cfg!(target_os = "windows") {
                    "pythonw"
                } else {
                    python_cmd
                };
                
                let mut child = Command::new(pythonw_cmd)
                    .arg("-u") // 禁用缓冲
                    .arg(&sqlmap_path)
                    .args(&args)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .env("PYTHONUNBUFFERED", "1") // 禁用缓冲
                    .spawn();
                
                // 存储进程ID以便后续可以停止
                if let Ok(ref child) = child {
                    if let Ok(mut process) = state_clone.current_process.lock() {
                        // 存储进程ID
                        *process = Some(child.id());
                    }
                }
                
                match child {
                    Ok(mut child) => {
                        // 创建子线程来实时读取输出
                        let stdout = child.stdout.take();
                        let stderr = child.stderr.take();
                        let stdin = child.stdin.take();
                        
                        let stdout_thread = {
                            let state_clone = state_clone.clone();
                            thread::spawn(move || {
                                if let Some(stdout) = stdout {
                                    let mut reader = std::io::BufReader::new(stdout);
                                    let mut line = String::new();
                                    while reader.read_line(&mut line).is_ok() {
                                        if !line.is_empty() {
                                            if let Ok(mut global_output) = state_clone.output.write() {
                                                global_output.push_str(&line);
                                            }
                                            line.clear();
                                        }
                                        // 短暂休眠以避免过度占用CPU
                                        thread::sleep(Duration::from_millis(10));
                                    }
                                }
                            })
                        };
                        
                        // 创建子线程来实时读取错误输出
                        let stderr_thread = {
                            let state_clone = state_clone.clone();
                            thread::spawn(move || {
                                if let Some(stderr) = stderr {
                                    let mut reader = std::io::BufReader::new(stderr);
                                    let mut line = String::new();
                                    while reader.read_line(&mut line).is_ok() {
                                        if !line.is_empty() {
                                            if let Ok(mut global_output) = state_clone.output.write() {
                                                global_output.push_str(&format!("错误: {}", line));
                                            }
                                            line.clear();
                                        }
                                        thread::sleep(Duration::from_millis(10));
                                    }
                                }
                            })
                        };
                        
                        // 创建子线程来监控进程状态并处理交互
                        let interaction_thread = {
                            let state_clone = state_clone.clone();
                            thread::spawn(move || {
                                // 存储stdin以便后续交互
                                if let Ok(mut global_stdin) = state_clone.child_stdin.lock() {
                                    *global_stdin = stdin;
                                }
                                
                                // 监控交互输入
                                let mut input_timeout_counter = 0;
                                loop {
                                    thread::sleep(Duration::from_millis(100));
                                    
                                    // 检查是否有待处理的交互输入
                                    if let Ok(pending) = state_clone.interaction_pending.read() {
                                        if *pending {
                                            input_timeout_counter += 1;
                                            
                                            // 检查超时 (5秒超时: 50 * 100ms)
                                            if input_timeout_counter > 50 {
                                                // 超时处理：重置状态并记录错误
                                                if let Ok(mut global_pending) = state_clone.interaction_pending.write() {
                                                    *global_pending = false;
                                                }
                                                if let Ok(mut global_output) = state_clone.output.write() {
                                                    global_output.push_str("\n[警告] 交互输入超时，可能SQLmap进程未在等待输入");
                                                }
                                                input_timeout_counter = 0;
                                                continue;
                                            }
                                            
                                            if let Ok(input) = state_clone.interaction_input.read() {
                                                if !input.is_empty() {
                                                    // 获取stdin并发送输入
                                                    if let Ok(mut global_stdin) = state_clone.child_stdin.lock() {
                                                        if let Some(ref mut stdin) = *global_stdin {
                                                            let input_with_newline = format!("{}\n", input);
                                                            match stdin.write_all(input_with_newline.as_bytes()) {
                                                                Ok(_) => {
                                                                    if let Err(e) = stdin.flush() {
                                                                        if let Ok(mut global_output) = state_clone.output.write() {
                                                                            global_output.push_str(&format!("\n[错误] 刷新输入失败: {}", e));
                                                                        }
                                                                    }
                                                                    
                                                                    // 成功发送后重置超时计数器
                                                                    input_timeout_counter = 0;
                                                                }
                                                                Err(e) => {
                                                                    if let Ok(mut global_output) = state_clone.output.write() {
                                                                        global_output.push_str(&format!("\n[错误] 发送输入失败: {}", e));
                                                                    }
                                                                }
                                                            }
                                                        } else {
                                                            // 没有有效的stdin句柄
                                                            if let Ok(mut global_output) = state_clone.output.write() {
                                                                global_output.push_str("\n[错误] 无法获取SQLmap进程的输入句柄");
                                                            }
                                                        }
                                                    }
                                                    
                                                    // 清空输入并重置状态
                                                    if let Ok(mut global_input) = state_clone.interaction_input.write() {
                                                        global_input.clear();
                                                    }
                                                    if let Ok(mut global_pending) = state_clone.interaction_pending.write() {
                                                        *global_pending = false;
                                                    }
                                                }
                                            }
                                        } else {
                                            // 没有待处理的输入，重置超时计数器
                                            input_timeout_counter = 0;
                                        }
                                    }
                                    
                                    // 检查进程是否仍在运行
                                    if let Ok(status) = child.try_wait() {
                                        if status.is_some() {
                                            break;
                                        }
                                    }
                                }
                                
                                // 等待进程完成
                                let _ = child.wait();
                                
                                // 等待输出线程完成
                                stdout_thread.join().ok();
                                stderr_thread.join().ok();
                                
                                // 清理stdin引用
                                if let Ok(mut global_stdin) = state_clone.child_stdin.lock() {
                                    *global_stdin = None;
                                }
                                
                                // 更新全局状态
                                if let Ok(mut global_output) = state_clone.output.write() {
                                    global_output.push_str("\nSQLmap扫描完成");
                                }
                                
                                if let Ok(mut global_running) = state_clone.is_running.write() {
                                    *global_running = false;
                                }
                            })
                        };
                        
                        // 不等待交互线程，让它后台运行
                        // interaction_thread会在后台自动运行，不需要显式detach
                    },
                    Err(e) => {
                        if let Ok(mut global_output) = state_clone.output.write() {
                            global_output.push_str(&format!("执行失败: {}", e));
                        }
                        
                        if let Ok(mut global_running) = state_clone.is_running.write() {
                            *global_running = false;
                        }
                    }
                }
                
                // 清理进程引用
                if let Ok(mut process) = state_clone.current_process.lock() {
                    *process = None;
                }
                
                // 更新全局状态
                if let Ok(mut global_output) = state_clone.output.write() {
                    global_output.push_str("\nSQLmap扫描完成");
                }
                
                if let Ok(mut global_running) = state_clone.is_running.write() {
                    *global_running = false;
                }
            });
        }
    };
    
    // 停止SQLmap
    let stop_sqlmap = { 
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        
        move |_| {
            // 尝试终止进程
            if let Ok(mut process) = state.current_process.lock() {
                if let Some(pid) = process.take() {
                    // 在Windows上使用taskkill命令停止进程
                    use std::process::Command;
                    let _ = Command::new("taskkill")
                        .args(["/F", "/PID", &pid.to_string()])
                        .spawn();
                }
            }
            
            // 更新全局状态
            if let Ok(mut global_running) = state.is_running.write() {
                *global_running = false;
            }
            
            if let Ok(mut global_output) = state.output.write() {
                global_output.push_str("\n已停止SQLmap");
            }
            
            // 立即刷新UI
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
        }
    };
    
    // 标签页状态类名
    let config_tab_class = if *active_tab.get() == "config" { "active" } else { "" };
    let other_config_tab_class = if *active_tab.get() == "other_config" { "active" } else { "" };
    
    // 切换标签页的闭包
    let switch_to_config = { 
        let active_tab = active_tab.clone();
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        
        move |_| {
            active_tab.set("config");
            // 切换标签页时刷新UI
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
        }
    };
    
    let switch_to_other_config = { 
        let active_tab = active_tab.clone();
        let state = state.clone();
        let config = config.clone();
        let is_running = is_running.clone();
        let output = output.clone();
        
        move |_| {
            active_tab.set("other_config");
            // 切换标签页时刷新UI
            if let Ok(global_config) = state.config.read() {
                config.set(global_config.clone());
            }
            
            if let Ok(global_running) = state.is_running.read() {
                is_running.set(*global_running);
            }
            
            if let Ok(global_output) = state.output.read() {
                output.set(global_output.clone());
            }
        }
    };
    
    // 刷新按钮专用的刷新闭包 - 为每个使用位置创建独立的闭包
    
    // 渲染主界面
    cx.render(rsx! {
        // 添加内联样式引用
        link {
            rel: "stylesheet",
            href: "style.css"
        }
        
        div {
            class: "main-container",
            "data-theme": "{theme}",
            
            // 头部 - 自定义标题栏
            header {
                class: "custom-titlebar",
                div {
                    class: "titlebar-content",
                    onmousedown: move |_| {
                        // 开始拖动窗口
                        let window = dioxus_desktop::use_window(&cx);
                        let _ = window.drag_window();
                    },
                    div {
                        class: "titlebar-left-section",
                        div {
                            class: "titlebar-icon",
                            dangerous_inner_html: "<svg width='32' height='32' viewBox='0 0 32 32' fill='none' xmlns='http://www.w3.org/2000/svg'><rect width='32' height='32' rx='6' fill='black'/><path d='M16 8C11.58 8 8 11.58 8 16C8 20.42 11.58 24 16 24C20.42 24 24 20.42 24 16C24 11.58 20.42 8 16 8ZM16 22C12.69 22 10 19.31 10 16C10 12.69 12.69 10 16 10C19.31 10 22 12.69 22 16C22 19.31 19.31 22 16 22Z' fill='white'/><path d='M16 12C13.79 12 12 13.79 12 16C12 18.21 13.79 20 16 20C18.21 20 20 18.21 20 16C20 13.79 18.21 12 16 12ZM16 18C14.9 18 14 17.1 14 16C14 14.9 14.9 14 16 14C17.1 14 18 14.9 18 16C18 17.1 17.1 18 16 18Z' fill='white'/><path d='M16 15C15.45 15 15 15.45 15 16C15 16.55 15.45 17 16 17C16.55 17 17 16.55 17 16C17 15.45 16.55 15 16 15Z' fill='white'/></svg>"
                        }
                        div {
                            class: "titlebar-text",
                            "SQLMAP"
                        }
                    }
                }
                div {
                    class: "titlebar-drag-region",
                }
                div {
                    class: "window-controls",
                    button {
                        class: "window-control theme-toggle",
                        style: "display: none;",
                        onclick: {
                            let state = state.clone();
                            let theme = theme.clone();
                            move |_| {
                                // 切换主题
                                if let Ok(mut global_theme) = state.theme.write() {
                                    let new_theme = if *global_theme == "light" {
                                        "dark".to_string()
                                    } else {
                                        "light".to_string()
                                    };
                                    *global_theme = new_theme.clone();
                                    theme.set(new_theme);
                                }
                            }
                        },
                        dangerous_inner_html: r#"
                            <svg class="sun-icon" width='12' height='12' viewBox='0 0 12 12' fill='currentColor'><circle cx='6' cy='6' r='3'/><path d='M6 1v1M6 10v1M1 6h1M10 6h1M2.5 2.5l.7.7M8.8 8.8l.7.7M2.5 9.5l.7-.7M8.8 3.2l.7-.7' stroke='currentColor' stroke-width='0.5' fill='none'/></svg>
                            <svg class="moon-icon" width='12' height='12' viewBox='0 0 12 12' fill='currentColor'><path d='M9.5 6.5c0 2.5-2 4.5-4.5 4.5S.5 9 .5 6.5c0-2 1.5-3.5 3.5-4-1 1-1.5 2.5-1.5 4 0 3 2.5 5.5 5.5 5.5s5.5-2.5 5.5-5.5c0-1.5-.5-3-1.5-4-2 .5-3.5 2-3.5 4z'/><path d='M6 1.5c.3 0 .5.2.5.5s-.2.5-.5.5-.5-.2-.5-.5.2-.5.5-.5z' fill='currentColor'/></svg>
                        "#
                    }
                    button {
                        class: "window-control minimize",
                        onclick: move |_| {
                            // 最小化窗口
                            let window = dioxus_desktop::use_window(&cx);
                            let _ = window.set_minimized(true);
                        },
                        dangerous_inner_html: "<svg width='12' height='12' viewBox='0 0 12 12' fill='currentColor'><rect x='2' y='5' width='8' height='2' rx='1'/></svg>"
                    }
                    button {
                        class: "window-control maximize",
                        onclick: move |_| {
                            // 最大化/还原窗口
                            let window = dioxus_desktop::use_window(&cx);
                            if window.is_maximized() {
                                let _ = window.set_maximized(false);
                            } else {
                                let _ = window.set_maximized(true);
                            }
                        },
                        dangerous_inner_html: "<svg width='12' height='12' viewBox='0 0 12 12' fill='currentColor'><rect x='1' y='1' width='10' height='10' rx='1' stroke='currentColor' stroke-width='1' fill='none'/></svg>"
                    }
                    button {
                        class: "window-control close",
                        onclick: move |_| {
                            // 关闭窗口
                            let window = dioxus_desktop::use_window(&cx);
                            let _ = window.close();
                        },
                        dangerous_inner_html: "<svg width='12' height='12' viewBox='0 0 12 12' fill='currentColor'><path d='M3 3L9 9M3 9L9 3' stroke='currentColor' stroke-width='1.5' stroke-linecap='round'/></svg>"
                    }
                }
            }
            
            // 主内容区域 - 左中右三栏布局
            div {
                class: "main-content",
                
                // 左侧：大面积显示输出信息
                div {
                    class: "left-panel",
                    div {
                        class: "left-panel-header",
                        div {
                            class: "output-header-row",
                            h3 { "扫描输出" }
                            button {
                                class: "refresh-button",
                                onclick: {
                                    let state = state.clone();
                                    let output = output.clone();
                                    move |_| {
                                        // 手动刷新输出：从全局状态重新加载输出内容
                                        if let Ok(global_output) = state.output.read() {
                                            output.set(global_output.clone());
                                        }
                                    }
                                },
                                "🔄 刷新"
                            }
                        }
                    }
                    div {
                        class: "output-container",
                        pre {
                            class: "output-content",
                            id: "output-content",
                            "{output}"
                        }
                    }

                    
                    // 交互输入区域
                    div {
                        class: "interaction-panel",
                        div {
                            class: "interaction-header",
                            h3 { "交互输入" }
                            p { "当SQLmap需要输入时（如y/n确认），在此输入并点击发送" }
                        }
                        div {
                            class: "interaction-input-group",
                            input {
                                r#type: "text",
                                placeholder: "输入y/n或其他命令...",
                                oninput: {
                                    let state = state.clone();
                                    move |e| {
                                        if let Ok(mut global_input) = state.interaction_input.write() {
                                            *global_input = e.value.clone();
                                        }
                                    }
                                },
                            }
                            button {
                                onclick: {
                                    let state = state.clone();
                                    move |_| {
                                        // 检查是否有正在运行的SQLmap进程和有效的stdin句柄
                                        let can_interact = {
                                            let running_ok = state.is_running.read().map(|r| *r).unwrap_or(false);
                                            let stdin_ok = state.child_stdin.lock().map(|s| s.is_some()).unwrap_or(false);
                                            running_ok && stdin_ok
                                        };
                                        
                                        if !can_interact {
                                            // 如果没有运行中的进程或无效的stdin句柄，显示提示信息
                                            if let Ok(mut global_output) = state.output.write() {
                                                global_output.push_str("\n错误：SQLmap进程未运行或无法接收交互输入");
                                            }
                                            return;
                                        }
                                        
                                        // 检查输入是否为空
                                        if let Ok(input) = state.interaction_input.read() {
                                            if input.trim().is_empty() {
                                                // 如果输入为空，显示提示信息
                                                if let Ok(mut global_output) = state.output.write() {
                                                    global_output.push_str("\n错误：请输入有效的交互命令");
                                                }
                                                return;
                                            }
                                        }
                                        
                                        // 设置交互待处理标志
                                        if let Ok(mut global_pending) = state.interaction_pending.write() {
                                            *global_pending = true;
                                        }
                                        
                                        // 记录发送的输入
                                        if let Ok(input) = state.interaction_input.read() {
                                            if let Ok(mut global_output) = state.output.write() {
                                                global_output.push_str(&format!("\n[用户输入] {}", input));
                                            }
                                        }
                                        
                                        // 清空输入框
                                        if let Ok(mut global_input) = state.interaction_input.write() {
                                            *global_input = String::new();
                                        }
                                    }
                                },
                                "发送"
                            }
                        }
                    }
                }
                
                // 中间：枚举选项和运行控制
                div {
                    class: "middle-panel",
                    
                    // 枚举选项
                    section {
                        h2 { "枚举选项" }
                        div {
                            class: "checkbox-group",
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.current_db}",
                                    onchange: move |e| update_current_db(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "当前数据库" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.current_user}",
                                    onchange: move |e| update_current_user(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "当前用户" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.current_user_dba}",
                                    onchange: move |e| update_current_user_dba(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "DBA权限" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.enumerate_dbs}",
                                    onchange: move |e| update_enumerate_dbs(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "枚举库名" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.enumerate_tables}",
                                    onchange: move |e| update_enumerate_tables(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "枚举表名" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.enumerate_columns}",
                                    onchange: move |e| update_enumerate_columns(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "枚举列名" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.enumerate_fields}",
                                    onchange: move |e| update_enumerate_fields(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "枚举字段" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.dump_all}",
                                    onchange: move |e| update_dump_all(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "一键脱库" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.os_shell}",
                                    onchange: move |e| update_os_shell(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "OS Shell" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.sql_shell}",
                                    onchange: move |e| update_sql_shell(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "SQL Shell" }
                            }
                            div {
                                class: "checkbox-item",
                                input {
                                    r#type: "checkbox",
                                    checked: "{config.purge_cache}",
                                    onchange: move |e| update_purge_cache(e.data.value.parse().unwrap_or(false)),
                                }
                                label { "清除缓存" }
                            }
                        }
                        
                        // 运行SQLmap
                        div {
                            class: "nav-buttons",
                            button {
                                onclick: run_sqlmap,
                                disabled: **is_running,
                                "运行SQLmap"
                            }
                            button {
                                onclick: stop_sqlmap,
                                disabled: !**is_running,
                                "停止"
                            }
                        }
                    }
                }
                
                // 右侧：配置和功能区域
                div {
                    class: "right-panel",
                    
                    // 右侧标签页导航
                    div {
                        class: "right-tabs",
                        button {
                            class: config_tab_class,
                            onclick: switch_to_config,
                            "配置"
                        }
                        button {
                            class: other_config_tab_class,
                            onclick: switch_to_other_config,
                            "其他配置"
                        }
                    }
                    
                    // 右侧标签页内容
                    div {
                        class: "right-content",
                        
                        // 配置内容
                        if *active_tab.get() == "config" {
                            rsx! {
                                div {
                                    class: "config-panel",
                                    
                                    // 目标设置
                                    section {
                                        h2 { "目标设置" }
                                        div {
                                            class: "form-group",
                                            label { "目标(URL或数据包): " }
                                            textarea {
                                                placeholder: "",
                                                value: "{config.target_url}",
                                                oninput: move |e| update_target_url(e.value.clone()),
                                                rows: "18",
                                                style: "width: 100%; resize: vertical;"
                                            }
                                        }
                                    }
                                    

                                    
                                    
                                    
                                    // 指定库名、表名、列名设置
                                    section {
                                        h2 { "指定目标" }
                                        div {
                                            class: "form-group",
                                            label { "库名: " }
                                            input {
                                                r#type: "text",
                                                placeholder: "指定数据库名称",
                                                value: "{config.database_name}",
                                                oninput: move |e| update_database_name(e.value.clone()),
                                            }
                                        }
                                        div {
                                            class: "form-group",
                                            label { "表名: " }
                                            input {
                                                r#type: "text",
                                                placeholder: "指定表名称",
                                                value: "{config.table_name}",
                                                oninput: move |e| update_table_name(e.value.clone()),
                                            }
                                        }
                                        div {
                                            class: "form-group",
                                            label { "列名: " }
                                            input {
                                                r#type: "text",
                                                placeholder: "指定列名称",
                                                value: "{config.column_name}",
                                                oninput: move |e| update_column_name(e.value.clone()),
                                            }
                                        }
                                        p {
                                            style: "font-size: 12px; color: var(--text-secondary); margin-top: 8px;",
                                            "使用这些字段可以精确指定要扫描的数据库、表或列。留空表示自动检测。"
                                        }
                                    }

                                    // 扫描设置
                                    section {
                                        h2 { "扫描设置" }
                                        div {
                                            class: "form-group",
                                            label { "扫描级别: " }
                                            input {
                                                r#type: "number",
                                                value: "{config.level}",
                                                oninput: move |e| update_level(e.value.clone()),
                                            }
                                        }
                                        div {
                                            class: "form-group",
                                            label { "风险级别: " }
                                            input {
                                                r#type: "number",
                                                value: "{config.risk}",
                                                oninput: move |e| update_risk(e.value.clone()),
                                            }
                                        }
                                        div {
                                            class: "form-group",
                                            label { "延迟(ms): " }
                                            input {
                                                r#type: "number",
                                                value: "{config.delay}",
                                                oninput: move |e| update_delay(e.value.clone()),
                                            }
                                        }
                                        div {
                                            class: "form-group",
                                            label { "详细度: " }
                                            input {
                                                r#type: "number",
                                                value: "{config.verbosity}",
                                                oninput: move |e| update_verbosity(e.value.clone()),
                                            }
                                        }
                                    }


                                }
                            }
                        }
                        
                        // 其他配置内容
                        else if *active_tab.get() == "other_config" {
                            // 定义tamper文件描述信息
                            let tamper_descriptions = vec![
                                ("space2comment", "将空格替换为注释/**/，绕过弱WAF"),
                                ("apostrophemask", "将单引号替换为UTF-8全角字符，绕过引号检测"),
                                ("base64encode", "对整个payload进行Base64编码"),
                                ("between", "用BETWEEN替换大于号，绕过比较操作符检测"),
                                ("charencode", "对payload进行URL编码"),
                                ("charunicodeencode", "使用Unicode编码字符"),
                                ("equaltolike", "用LIKE替换等号，绕过等号检测"),
                                ("greatest", "用GREATEST替换大于号"),
                                ("ifnull2ifisnull", "用IF(ISNULL(A))替换IFNULL(A)"),
                                ("modsecurityversioned", "添加版本化注释，绕过ModSecurity"),
                                ("modsecurityzeroversioned", "添加零版本注释，绕过ModSecurity"),
                                ("percentage", "在每个字符前添加百分号"),
                                ("randomcase", "随机大小写字符"),
                                ("randomcomments", "在关键字中插入随机注释"),
                                ("space2dash", "用破折号注释替换空格"),
                                ("space2hash", "用井号注释替换空格"),
                                ("space2morehash", "用多个井号注释替换空格"),
                                ("space2mssqlblank", "用随机空白字符替换空格(MSSQL)"),
                                ("space2mysqlblank", "用随机空白字符替换空格(MySQL)"),
                                ("space2plus", "用加号替换空格"),
                                ("unionalltounion", "用UNION替换UNION ALL"),
                                ("unmagicquotes", "用字符替换魔术引号"),
                                ("versionedkeywords", "在关键字前插入版本注释"),
                                ("versionedmorekeywords", "在更多关键字前插入版本注释"),
                                ("xforwardedfor", "添加X-Forwarded-For头"),
                            ];
                            
                            // 计算tamper统计信息
                            let total_count = 70; // 总tamper数量
                            let selected_count = config.selected_tampers.len();
                            
                            rsx! {
                                div {
                                    class: "other-config-panel",
                                    style: "width: 100%; max-width: none;",
                                    
                                    // 绕过模板(tamper)设置 - 改为按钮
                            div {
                                class: "tamper-settings",
                                style: "margin-bottom: 20px; padding: 16px; background: var(--bg-secondary); border-radius: 8px; width: 100%; max-width: none;",
                                
                                h4 { 
                                    style: "font-size: 16px; margin-bottom: 10px; color: var(--text-primary);",
                                    "绕过模板(Tamper)设置" 
                                }
                                
                                button {
                                    class: "tamper-overlay-btn",
                                    style: "width: 100%; padding: 12px; background: var(--accent-color); color: var(--text-primary); border: none; border-radius: 6px; font-size: 14px; cursor: pointer; transition: background-color 0.2s;",
                                    onclick: move |_| {
                                        show_tamper_overlay.set(true);
                                    },
                                    "配置绕过模板 ({selected_count}/{total_count})"
                                }
                                
                                p {
                                    style: "font-size: 12px; color: var(--text-secondary); margin-top: 10px;"
                                }
                            }
                                    
                                    // 自定义参数和数据库类型设置（扩大宽度）
                                    div {
                                        class: "custom-settings",
                                        style: "margin-top: 0; padding: 25px; background: var(--bg-secondary); border-radius: 8px; width: 100%; max-width: none;",
                                        
                                        // 上下排列布局 - 增加宽度
                                        div {
                                            class: "settings-vertical",
                                            style: "display: flex; flex-direction: column; gap: 30px; width: 100%;",
                                            
                                            // 自定义参数设置 - 上方 - 增加宽度
                                            section {
                                                style: "width: 100%; max-width: none;",
                                                h4 { 
                                                    style: "font-size: 18px; margin-bottom: 15px; color: var(--text-primary);",
                                                    "自定义参数设置" 
                                                }
                                                div {
                                                    class: "form-group",
                                                    style: "width: 100%; max-width: none;",
                                                    label { 
                                                        style: "font-weight: bold; margin-bottom: 8px; display: block;",
                                                        "自定义参数: " 
                                                    }
                                                    textarea {
                                                        placeholder: "例如: --batch --threads=5 --tamper=space2comment",
                                                        value: "{config.custom_parameters}",
                                                        oninput: move |e| update_custom_parameters(e.value.clone()),
                                                        rows: "16",
                                                        style: "width: 100%; max-width: none; resize: vertical; min-height: 100px; padding: 12px; border: 1px solid var(--border-color); border-radius: 4px; font-size: 14px;"
                                                    }
                                                    p {
                                                        style: "font-size: 12px; color: var(--text-secondary); margin-top: 8px;",
                                                        "输入额外的SQLmap参数，多个参数用空格分隔"
                                                    }
                                                }
                                            }
                                            
                                            // 数据库类型设置 - 下方 - 增加宽度
                                            section {
                                                style: "width: 100%; max-width: none;",
                                                h4 { 
                                                    style: "font-size: 18px; margin-bottom: 15px; color: var(--text-primary);", 
                                                    "数据库类型设置" 
                                                }
                                                div {
                                                    class: "form-group",
                                                    style: "width: 100%; max-width: none;",
                                                    label { 
                                                        style: "font-weight: bold; margin-bottom: 8px; display: block;",
                                                        "数据库类型: " 
                                                    }
                                                    select {
                                                        value: "{config.dbms_type}",
                                                        onchange: move |e| update_dbms_type(e.value.clone()),
                                                        style: "width: 100%; max-width: none; padding: 12px; border-radius: 4px; border: 1px solid var(--border-color); font-size: 14px; background: white; min-height: 44px;",
                                                        option { value: "auto", "自动检测" }
                                                        option { value: "altibase", "Altibase" }
                                                        option { value: "amazon_redshift", "Amazon Redshift" }
                                                        option { value: "apache_derby", "Apache Derby" }
                                                        option { value: "apache_ignite", "Apache Ignite" }
                                                        option { value: "aurora", "Aurora" }
                                                        option { value: "clickhouse", "ClickHouse" }
                                                        option { value: "cockroachdb", "CockroachDB" }
                                                        option { value: "cratedb", "CrateDB" }
                                                        option { value: "cubrid", "Cubrid" }
                                                        option { value: "drizzle", "Drizzle" }
                                                        option { value: "enterprisedb", "EnterpriseDB" }
                                                        option { value: "extremedb", "eXtremeDB" }
                                                        option { value: "firebird", "Firebird" }
                                                        option { value: "frontbase", "FrontBase" }
                                                        option { value: "greenplum", "Greenplum" }
                                                        option { value: "h2", "H2" }
                                                        option { value: "hsqldb", "HSQLDB" }
                                                        option { value: "ibm_db2", "IBM DB2" }
                                                        option { value: "informix", "Informix" }
                                                        option { value: "intersystems_cache", "InterSystems Cache" }
                                                        option { value: "iris", "Iris" }
                                                        option { value: "mariadb", "MariaDB" }
                                                        option { value: "mckoi", "Mckoi" }
                                                        option { value: "memsql", "MemSQL" }
                                                        option { value: "access", "Microsoft Access" }
                                                        option { value: "mssql", "Microsoft SQL Server" }
                                                        option { value: "mimersql", "MimerSQL" }
                                                        option { value: "monetdb", "MonetDB" }
                                                        option { value: "mysql", "MySQL" }
                                                        option { value: "opengauss", "OpenGauss" }
                                                        option { value: "oracle", "Oracle" }
                                                        option { value: "percona", "Percona" }
                                                        option { value: "postgresql", "PostgreSQL" }
                                                        option { value: "presto", "Presto" }
                                                        option { value: "raima_dbm", "Raima Database Manager" }
                                                        option { value: "maxdb", "SAP MaxDB" }
                                                        option { value: "sqlite", "SQLite" }
                                                        option { value: "sybase", "Sybase" }
                                                        option { value: "tidb", "TiDB" }
                                                        option { value: "vertica", "Vertica" }
                                                        option { value: "virtuoso", "Virtuoso" }
                                                        option { value: "yellowbrick", "Yellowbrick" }
                                                        option { value: "yugabytedb", "YugabyteDB" }
                                                    }
                                                    p {
                                                        style: "font-size: 12px; color: var(--text-secondary); margin-top: 8px;",
                                                        "指定目标数据库类型，提高扫描效率"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            // 状态栏
            footer {
                class: "status-bar",
                span { "SQLmap GUI v0.1.0" }
                span {
                    if **is_running { "运行中..." } else { "就绪" }
                }
            }
        }
        
        // Tamper设置覆盖页面
        if **show_tamper_overlay {
            // 获取tamper描述
            let get_tamper_description = |tamper: &str| -> &'static str {
                match tamper {
                    "0eunion" => "用 0eunion 替换 UNION，绕过 UNION 检测",
                    "apostrophemask" => "使用 UTF-8 全角字符替换单引号 (') 进行混淆",
                    "apostrophenullencode" => "使用非法双字节 Unicode 字符替换单引号 (') 进行编码混淆",
                    "appendnullbyte" => "在有效载荷末尾附加空字节（NULL byte）字符编码",
                    "base64encode" => "对 Payload 进行 Base64 编码",
                    "between" => "使用 BETWEEN 替换比较操作符（如 >），绕过比较操作符检测",
                    "binary" => "使用二进制字符串格式替换字符（如将 'abc' 替换为 0x616263）",
                    "bluecoat" => "使用随机空白字符替换空格，并在其后添加 'LIKE'，以绕过 BlueCoat WAF",
                    "chardoubleencode" => "对给定的 Payload 进行双重 URL 编码",
                    "charencode" => "对 Payload 进行 URL 编码",
                    "charunicodeencode" => "使用 Unicode 编码（如 ' 替换 '）对字符进行编码",
                    "charunicodeescape" => "使用 Unicode 转义序列（如 \\u0027 替换 '）对字符进行编码",
                    "commalesslimit" => "使用偏移量语法替换 LIMIT M, N（如 LIMIT N OFFSET M），绕过逗号检测",
                    "commalessmid" => "使用 FROM 替换 MID () 函数中的逗号（如 MID (col FROM 1 FOR 10)）",
                    "commentbeforeparentheses" => "在括号前插入注释（如 /comment/(）",
                    "concat2concatws" => "使用 CONCAT_WS () 函数替换 CONCAT () 函数（如 CONCAT_WS ('', 'a', 'b')）",
                    "decentities" => "使用十进制 HTML 实体（如 ' 替换 '）替换字符",
                    "dunion" => "使用 DUNION 替换 UNION 关键字",
                    "equaltolike" => "使用 LIKE 替换等号（=），绕过等号检测（如 a LIKE b 替换 a = b）",
                    "equaltorlike" => "使用 RLIKE 替换等号（=）（如 a RLIKE b 替换 a = b）",
                    "escapequotes" => "使用转义符对引号进行转义（如用 \\' 替换单引号，用 \\\" 替换双引号）",
                    "greatest" => "使用 GREATEST () 函数替换大于号（>）（如 GREATEST (a, b+1) 替换 a > b）",
                    "halfversionedmorekeywords" => "在关键字前插入部分版本注释（如 /!50000UNION/）",
                    "hex2char" => "使用 CHAR () 函数替换十六进制编码（如 CHAR (0x61) 替换 'a'）",
                    "hexentities" => "使用十六进制 HTML 实体（如 ' 替换 '）替换字符",
                    "htmlencode" => "对 Payload 进行 HTML 编码（如 < 替换 <）",
                    "if2case" => "使用 CASE 语句替换 IF () 函数（如 CASE WHEN a THEN b ELSE c END 替换 IF (a, b, c)）",
                    "ifnull2casewhenisnull" => "使用 CASE WHEN ISNULL () 替换 IFNULL () 函数（如 CASE WHEN ISNULL (a) THEN b ELSE a END 替换 IFNULL (a, b)）",
                    "ifnull2ifisnull" => "使用 IF (ISNULL ()) 替换 IFNULL () 函数（如 IF (ISNULL (a), b, a) 替换 IFNULL (a, b)）",
                    "informationschemacomment" => "在 information_schema 后插入注释（如 information_schema/comment/.table）",
                    "least" => "使用 LEAST () 函数替换小于号（<）（如 LEAST (a, b-1) 替换 a < b）",
                    "lowercase" => "将 SQL 关键字转换为小写（如 union 替换 UNION）",
                    "luanginx" => "绕过 Lua+Nginx Web 应用防火墙（WAF）的检测",
                    "luanginxmore" => "绕过 Lua+Nginx Web 应用防火墙（WAF）的检测（更多变体）",
                    "misunion" => "使用 MISUNION 替换 UNION 关键字",
                    "modsecurityversioned" => "添加版本化注释（如 /!40100 ... /）以绕过 ModSecurity WAF",
                    "modsecurityzeroversioned" => "添加零版本注释（如 /!0 ... /）以绕过 ModSecurity WAF",
                    "multiplespaces" => "在 SQL 关键字周围添加多个空格（如 U N I O N）",
                    "ord2ascii" => "使用 ORD () 函数替换 ASCII () 函数（如 ORD (a) 替换 ASCII (a)）",
                    "overlongutf8" => "使用超长 UTF-8 编码替换字符（如 % c0% a7 替换 '）",
                    "overlongutf8more" => "使用超长 UTF-8 编码替换字符（更多变体形式）",
                    "percentage" => "在每个字符前添加百分号（%）进行混淆（如 % u% n% i% o% n）",
                    "plus2concat" => "使用 CONCAT () 函数替换加号（+）运算符（如 CONCAT (a, b) 替换 a + b）",
                    "plus2fnconcat" => "使用函数形式的 CONCAT () 替换加号（+）运算符（如 fn:CONCAT (a, b) 替换 a + b）",
                    "randomcase" => "对 SQL 关键字进行随机大小写转换（如 UnIoN）",
                    "randomcomments" => "在 SQL 关键字中插入随机注释（如 U/rnd/N/cmt/I/.../ON）",
                    "schemasplit" => "使用点分割模式名称（如 information_schema.tables 替换 information_schema.tables，增强混淆）",
                    "scientific" => "使用科学计数法表示数字来替换空格（如 1e0 替换 1）",
                    "sleep2getlock" => "使用 GET_LOCK () 函数替换 SLEEP () 函数（如 GET_LOCK ('x', 5) 替换 SLEEP (5)）",
                    "sp_password" => "在有效载荷后追加 sp_password（SQL Server 特定，利用其特殊处理机制）",
                    "space2comment" => "使用注释替换空格（如 /* / 替换空格）",
                    "space2dash" => "使用破折号注释替换空格（如 -- 替换空格，需注意后续字符处理）",
                    "space2hash" => "使用井号（#）注释替换空格（MySQL 特定）",
                    "space2morecomment" => "使用多行注释替换空格（如 /**/ 替换空格）",
                    "space2morehash" => "使用多个井号（#）注释替换空格（MySQL 特定）",
                    "space2mssqlblank" => "使用随机空白字符（如 \\t、\\n）替换空格（针对 MSSQL）",
                    "space2mssqlhash" => "使用 MSSQL 实例注释字符替换空格（如 /%/）",
                    "space2mysqlblank" => "使用随机空白字符（如 \\t、\\n）替换空格（针对 MySQL）",
                    "space2mysqldash" => "使用破折号注释替换空格（如 -- - 替换空格，MySQL 特定）",
                    "space2plus" => "使用加号（+）替换空格",
                    "space2randomblank" => "使用随机空白字符（如 \\t、\\n、\\r）替换空格",
                    "substring2leftright" => "使用 LEFT () 和 RIGHT () 函数替换 SUBSTRING () 函数（如 LEFT (a, 1) 替换 SUBSTRING (a, 1, 1)）",
                    "symboliclogical" => "使用符号逻辑运算符替换 AND/OR（如 && 替换 AND，|| 替换 OR）",
                    "unionalltounion" => "使用 UNION 替换 UNION ALL（如 UNION SELECT 替换 UNION ALL SELECT）",
                    "unmagicquotes" => "对抗魔术引号（Magic Quotes）机制，使用字符替换转义的引号",
                    "uppercase" => "将 SQL 关键字转换为大写（如 UNION 替换 union）",
                    "varnish" => "绕过 Varnish 缓存 / 防火墙的检测",
                    "versionedkeywords" => "在关键字前插入版本注释（如 /!UNION*/）",
                    "versionedmorekeywords" => "在更多 SQL 关键字前插入版本注释（扩展 versionedkeywords 的覆盖范围）",
                    "xforwardedfor" => "添加 X-Forwarded-For 请求头以绕过某些 WAF 检测",
                    _ => "未知绕过模板"
                }
            };
            
            // 获取所有tamper文件列表
            let all_tampers = vec![
                "0eunion", "apostrophemask", "apostrophenullencode", "appendnullbyte", "base64encode", 
                "between", "binary", "bluecoat", "chardoubleencode", "charencode", "charunicodeencode", 
                "charunicodeescape", "commalesslimit", "commalessmid", "commentbeforeparentheses", 
                "concat2concatws", "decentities", "dunion", "equaltolike", "equaltorlike", "escapequotes", 
                "greatest", "halfversionedmorekeywords", "hex2char", "hexentities", "htmlencode", "if2case", 
                "ifnull2casewhenisnull", "ifnull2ifisnull", "informationschemacomment", "least", "lowercase", 
                "luanginx", "luanginxmore", "misunion", "modsecurityversioned", "modsecurityzeroversioned", 
                "multiplespaces", "ord2ascii", "overlongutf8", "overlongutf8more", "percentage", "plus2concat", 
                "plus2fnconcat", "randomcase", "randomcomments", "schemasplit", "scientific", "sleep2getlock", 
                "sp_password", "space2comment", "space2dash", "space2hash", "space2morecomment", "space2morehash", 
                "space2mssqlblank", "space2mssqlhash", "space2mysqlblank", "space2mysqldash", "space2plus", 
                "space2randomblank", "substring2leftright", "symboliclogical", "unionalltounion", "unmagicquotes", 
                "uppercase", "varnish", "versionedkeywords", "versionedmorekeywords", "xforwardedfor"
            ];
            
            // 过滤tamper列表
            let filtered_tampers: Vec<&str> = if tamper_search_query.is_empty() {
                all_tampers.iter().map(|s| *s).collect()
            } else {
                let query_lower = tamper_search_query.to_lowercase();
                all_tampers.iter()
                    .filter(|tamper| {
                        // 搜索tamper名称
                        let name_match = tamper.to_lowercase().contains(&query_lower);
                        // 搜索tamper描述
                        let description_match = get_tamper_description(tamper).to_lowercase().contains(&query_lower);
                        name_match || description_match
                    })
                    .map(|s| *s)
                    .collect()
            };
            
            let state_clone = state.clone();
            
            rsx! {
                // 背景遮罩
                div {
                    class: if **is_closing { "tamper_overlay_mask closing" } else { "tamper_overlay_mask" },
                    style: "position: fixed; top: 0; left: 0; right: 0; bottom: 0; background: rgba(0, 0, 0, 0.5); z-index: 1000; display: flex; justify-content: flex-end; align-items: stretch;",
                    onclick: move |_| {
                        is_closing.set(true);
                        // 延迟关闭，等待动画完成
                        let show_tamper_overlay = show_tamper_overlay.clone();
                        let tamper_search_query = tamper_search_query.clone();
                        let is_closing = is_closing.clone();
                        cx.spawn(async move {
                            time::sleep(Duration::from_millis(300)).await;
                            show_tamper_overlay.set(false);
                            tamper_search_query.set(String::new());
                            is_closing.set(false);
                        });
                    },
                    
                    // 内容容器 - 从右侧滑入
                    div {
                        class: if **is_closing { "tamper_overlay_content closing" } else { "tamper_overlay_content" },
                        style: "width: 90%; max-width: 1200px; height: 100%; background: var(--bg-primary); border-radius: 12px 0 0 12px; box-shadow: -2px 0 20px rgba(0, 0, 0, 0.3); display: flex; flex-direction: column;",
                        onclick: move |e| e.stop_propagation(),
                        
                        // 头部
                        div {
                            class: "tamper_overlay_header",
                            style: "padding: 20px; border-bottom: 1px solid var(--border-color); display: flex; justify-content: space-between; align-items: center; background: var(--bg-secondary); border-radius: 12px 0 0 0;",
                            
                            h2 {
                                style: "margin: 0; font-size: 24px; color: var(--text-primary);",
                                "绕过模板配置"
                            }
                            
                            button {
                                class: "window-control close",
                                onclick: move |_| {
                                    is_closing.set(true);
                                    // 延迟关闭，等待动画完成
                                    let show_tamper_overlay = show_tamper_overlay.clone();
                                    let tamper_search_query = tamper_search_query.clone();
                                    let is_closing = is_closing.clone();
                                    cx.spawn(async move {
                                        time::sleep(Duration::from_millis(300)).await;
                                        show_tamper_overlay.set(false);
                                        tamper_search_query.set(String::new());
                                        is_closing.set(false);
                                    });
                                },
                                dangerous_inner_html: "<svg width='12' height='12' viewBox='0 0 12 12' fill='currentColor'><path d='M3 3L9 9M3 9L9 3' stroke='currentColor' stroke-width='1.5' stroke-linecap='round'/></svg>"
                            }
                        }
                        
                        // 搜索栏
                        div {
                            class: "tamper_search_bar",
                            style: "padding: 20px; border-bottom: 1px solid var(--border-color); background: var(--bg-secondary);",
                            
                            div {
                                style: "display: flex; gap: 10px; align-items: center;",
                                
                                input {
                                    r#type: "text",
                                    placeholder: "搜索绕过模板...",
                                    value: "{tamper_search_query}",
                                    oninput: move |e| tamper_search_query.set(e.value.clone()),
                                    style: "flex: 1; padding: 12px; border: 1px solid var(--border-color); border-radius: 6px; font-size: 14px;"
                                }
                                
                                span {
                                    style: "font-size: 14px; color: var(--text-secondary); white-space: nowrap;",
                                    "找到 {filtered_tampers.len()} 个模板"
                                }
                            }
                        }
                        
                        // 模板列表
                        div {
                            class: "tamper_list_container",
                            style: "flex: 1; overflow-y: auto; padding: 20px;",
                            
                            div {
                                class: "tamper_grid",
                                style: "display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 15px;",
                                
                                {filtered_tampers.iter().map(|tamper| {
                                    let is_selected = config.selected_tampers.contains(&tamper.to_string());
                                    let tamper_clone = tamper.to_string();
                                    let description = get_tamper_description(tamper);
                                    let state_clone_for_card = state_clone.clone();
                                    
                                    rsx! {
                                        div {
                                            class: "tamper_card",
                                            style: if is_selected {
                                                "padding: 16px; border: 2px solid var(--accent-color); border-radius: 8px; background: var(--bg-secondary); cursor: pointer; transition: all 0.2s;"
                                            } else {
                                                "padding: 16px; border: 2px solid var(--border-color); border-radius: 8px; background: var(--bg-secondary); cursor: pointer; transition: all 0.2s;"
                                            },
                                            onclick: move |_| {
                                                let mut new_tampers = config.selected_tampers.clone();
                                                if is_selected {
                                                    new_tampers.retain(|x| x != &tamper_clone);
                                                } else {
                                                    new_tampers.push(tamper_clone.clone());
                                                }
                                                update_selected_tampers_fn(state_clone_for_card.clone(), &config, new_tampers);
                                            },
                                            
                                            div {
                                                style: "display: flex; align-items: flex-start; gap: 12px;",
                                                
                                                input {
                                                    r#type: "checkbox",
                                                    checked: is_selected,
                                                    style: "margin-top: 2px;",
                                                    onclick: move |e| e.stop_propagation(),
                                                }
                                                
                                                div {
                                                    style: "flex: 1;",
                                                    
                                                    strong {
                                                        style: "display: block; font-size: 16px; color: var(--text-primary); margin-bottom: 8px;",
                                                        "{tamper}"
                                                    }
                                                    
                                                    p {
                                                        style: "font-size: 14px; color: var(--text-secondary); line-height: 1.4; margin: 0;",
                                                        "{description}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                })}
                            }
                        }
                        
                        // 底部操作栏
                        div {
                            class: "tamper_overlay_footer",
                            style: "padding: 20px; border-top: 1px solid var(--border-color); background: var(--bg-secondary); border-radius: 0 0 0 12px; display: flex; justify-content: space-between; align-items: center;",
                            
                            span {
                                style: "font-size: 14px; color: var(--text-secondary);",
                                "已选择 {config.selected_tampers.len()} 个模板"
                            }
                            
                            div {
                                style: "display: flex; gap: 10px;",
                                
                                button {
                                    class: "clear_btn",
                                    style: "padding: 10px 20px; background: var(--accent-primary); color: white; border: none; border-radius: 6px; cursor: pointer; font-size: 14px;",
                                    onclick: move |_| {
                                        update_selected_tampers_fn(state_clone.clone(), &config, Vec::new());
                                    },
                                    "清空选择"
                                }
                                
                                button {
                                    class: "confirm_btn",
                                    style: "padding: 10px 20px; background: var(--accent-primary); color: white; border: none; border-radius: 6px; cursor: pointer; font-size: 14px;",
                                    onclick: move |_| {
                                        is_closing.set(true);
                                        // 延迟关闭，等待动画完成
                                        let show_tamper_overlay = show_tamper_overlay.clone();
                                        let tamper_search_query = tamper_search_query.clone();
                                        let is_closing = is_closing.clone();
                                        cx.spawn(async move {
                                            time::sleep(Duration::from_millis(300)).await;
                                            show_tamper_overlay.set(false);
                                            tamper_search_query.set(String::new());
                                            is_closing.set(false);
                                        });
                                    },
                                    "确认选择"
                                }
                            }
                        }
                    }
                }
            }
        } else {
            rsx! {
                div { style: "display: none;" }
            }
        }
    })
}
                                