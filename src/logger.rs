use simplelog::*;
use std::fs::{self, File};
use std::path::Path;

pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = Path::new("logs");
    if !log_dir.exists() {
        // ログディレクトリが存在しない場合は作成
        fs::create_dir_all(log_dir)?;
    }

    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let log_file_name = format!("world-server_{}.log", timestamp);
    let log_file_path = log_dir.join(log_file_name);

    // コンソール出力の設定
    let console_config = ConfigBuilder::new()
        .set_thread_level(LevelFilter::Debug)
        .set_target_level(LevelFilter::Info)
        .set_location_level(LevelFilter::Warn)
        .build();

    // ファイル出力の設定
    let file_config = ConfigBuilder::new()
        .set_thread_level(LevelFilter::Info)
        .set_target_level(LevelFilter::Info)
        .set_location_level(LevelFilter::Info)
        .build();

    // ロガーの初期化
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Info,
            console_config,
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(LevelFilter::Info, file_config, File::create(log_file_path)?),
    ])?;

    Ok(())
}
