use px_protocol::client;
use px_protocol::render::Request;

pub fn request(request: Request, autostart: bool) -> i32 {
    match client::request_with(request, autostart) {
        Ok(response) => {
            for shot in &response.shots {
                println!("写出：{shot}");
            }
            println!(
                "{} → {}（{}×{}，{} 字节，耗时 {} ms，{}，共 {} 张）",
                response.scene,
                response.out,
                response.width,
                response.height,
                response.bytes,
                response.millis,
                if response.warm {
                    "服务已热"
                } else {
                    "服务刚起"
                },
                response.shots.len().max(1),
            );
            if !response.report_path.is_empty() {
                println!("报告：{}", response.report_path);
            }
            if !response.report.is_empty() {
                println!("{}", response.report);
            }
            0
        }
        Err(px_protocol::render::ClientError::NoServer) => {
            eprintln!("没有在跑的渲染服务。先起一个：");
            eprintln!("    px_render --serve");
            eprintln!("租约文件：{}", client::lease_path().display());
            eprintln!("（要在本进程里直接出图就加 --offline，不必起服务）");
            1
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}
