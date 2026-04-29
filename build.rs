use std::fs;

use tonic_prost_build;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 出力ファイル・ディレクトリの作成
    fs::create_dir_all("src/generated/")?;
    let server_out_dir = "src/generated/server/";
    let client_out_dir = "src/generated/client/";
    let inner_out_dir = "src/generated/inner/";
    fs::create_dir_all(server_out_dir)?;
    fs::create_dir_all(client_out_dir)?;
    fs::create_dir_all(inner_out_dir)?;

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(inner_out_dir)
        .compile_protos(
            &[
                "process/lobby/service_lobby.proto",
                "process/world/command_zone.proto",
            ],
            &["portfolio-proto"],
        )?;

    // tonic_prost_build::configure()
    //     .build_client(false)
    //     .build_server(true)
    //     .out_dir(server_out_dir)
    //     .compile_protos(&["process/world/service_zone.proto"], &["portfolio-proto"])?;

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(false)
        .out_dir(client_out_dir)
        .compile_protos(
            &[
                "process/world/service_zone.proto",
                "process/db/user/service.proto",
                "process/db/record/service.proto",
            ],
            &["portfolio-proto"],
        )?;
    Ok(())
}
