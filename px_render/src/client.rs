//! 客户端那一半：把一条 `Request` 交给在跑的服务，把回话打给使用者。
//!
//! **连接/握手/超时全部走 `px_protocol::client`**（§102：协议不许在这里另抄一份）。
//! 这里只做两件事：把回话打成给人看的一行行，以及把错误翻成退出码。
//!
//! 逐字照搬 Bevy 宿主（`px_render/src/main.rs::request_once` :1078-1137）—— 包括那句
//! "没有在跑的渲染服务"后面跟的用法提示：仪器（`tools/harness.ps1::Invoke-Client`）
//! 判的是**退出码**，而人判的是这两行字。

use px_protocol::client as client;
use px_protocol::render::Request;

/// 发一条请求。返回进程退出码（0 成功）。
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
            // 报告也**回给调用方**（与落盘那份逐字节相同）：调用方不必再去读文件。
            if !response.report.is_empty() {
                println!("{}", response.report);
            }
            0
        }
        Err(px_protocol::render::ClientError::NoServer) => {
            eprintln!("没有在跑的渲染服务。先起一个：");
            eprintln!("    px_render --serve");
            eprintln!("租约文件：{}", client::lease_path().display());
            // ⚠ 这一句是给"其实想在本进程里出图"的人看的：少了它，拒词只说清了一半，
            //    而"没有服务"与"我不想走服务"是两件事（`--offline` 那条路一条命令就够）。
            eprintln!("（要在本进程里直接出图就加 --offline，不必起服务）");
            1
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}
