//! 经济世界这一侧的 crate。世界视图的**序列化形状**归这里，不归 `px_protocol`：
//! 那张协议只认 px-scene ⇄ px-pass 的交换，世界视图两边都不认。
pub mod sim;
