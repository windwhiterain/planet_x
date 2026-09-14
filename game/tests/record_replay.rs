use std::path::PathBuf;

use game::{DomesticEconomy, project};
use px_protocol::ProtocolId;
use px_protocol::stream::{self, Frame};

fn record_with(rounds: usize, seed: u64, fluctuation: f32) -> Vec<Frame> {
    let mut economy = DomesticEconomy::new(seed).with_fluctuation(fluctuation);
    economy.run(rounds);
    let mut frames = vec![Frame::Protocol(ProtocolId::local())];
    frames.extend(
        economy
            .history
            .iter()
            .map(|snapshot| Frame::World(project::world_view(snapshot))),
    );
    frames
}

fn record(rounds: usize, seed: u64) -> Vec<Frame> {
    record_with(rounds, seed, 0.0)
}

fn prices(frames: &[Frame]) -> Vec<f32> {
    frames
        .iter()
        .flat_map(|frame| match frame {
            Frame::World(world) => world.goods.iter().map(|good| good.price).collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn record_then_replay_is_byte_identical() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let recorded_path = dir.join("record.pxstream");
    let replay_path = dir.join("replay.pxstream");

    let frames = record(60, 11);
    let mut bytes = Vec::new();
    stream::write_stream(&mut bytes, &frames).expect("写流失败");
    std::fs::write(&recorded_path, &bytes).expect("落盘失败");

    let replayed = stream::read_stream(&mut std::fs::File::open(&recorded_path).expect("打开失败"))
        .expect("读流失败");
    assert_eq!(replayed, frames);

    let mut again = Vec::new();
    stream::write_stream(&mut again, &replayed).expect("重放写流失败");
    std::fs::write(&replay_path, &again).expect("落盘失败");
    assert_eq!(bytes, again, "录制 → 重放 → 再录制必须逐字节相同");
}

#[test]
fn recorded_rounds_are_dense_and_ordered() {
    let frames = record(12, 11);
    let rounds: Vec<u32> = frames
        .iter()
        .filter_map(|frame| match frame {
            Frame::World(world) => Some(world.round),
            _ => None,
        })
        .collect();
    assert_eq!(rounds, (0..=12).collect::<Vec<u32>>());
}

#[test]
fn the_first_frame_declares_the_protocol() {
    let frames = record(1, 11);
    assert!(matches!(frames.first(), Some(Frame::Protocol(_))));
}

#[test]
fn content_actually_varies_across_rounds() {
    let distinct: std::collections::BTreeSet<u32> = prices(&record(60, 11))
        .iter()
        .map(|price| price.to_bits())
        .collect();
    assert!(
        distinct.len() >= 5,
        "60 轮里商品价格只有 {} 个不同取值：录制内容像是冻住了",
        distinct.len()
    );
}

#[test]
fn zero_fluctuation_makes_the_seed_irrelevant() {
    assert_eq!(
        record(40, 11),
        record(40, 12),
        "波动为 0 时世界与 seed 无关。这是 sim 的既有性质；若这条失败，\
         说明 sim 开始在 f=0 时消费 RNG —— 那时基线口径要重新定"
    );
}

#[test]
fn same_seed_with_fluctuation_is_reproducible() {
    assert_eq!(record_with(40, 11, 0.4), record_with(40, 11, 0.4));
}

#[test]
fn fluctuation_makes_the_seed_matter() {
    assert_ne!(
        record_with(40, 11, 0.4),
        record_with(40, 12, 0.4),
        "开了波动之后不同 seed 必须给出不同的世界"
    );
}
