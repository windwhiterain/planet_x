# WebUI dev server 自动选空闲端口（`planet_x_web` 启动不再撞端口）

> 状态 `[x]`（branch `feature/web-auto-port`） ｜ 索引：[notes.md](../notes.md) ｜ 前身：`ideas.md` §24

- **动机**：本机同时开两个 WebUI（玩家一个、agent 一个做实机验证）时，第二个从前只会抛
  `os error 10048 地址已占用` 然后退出——而「再开一个」正是最常见的动作。

- `[x]` **`bind_auto(host, base, attempts)`**（`web/src/lib.rs`）：从 `base` 起**向上扫**，绑上第一个空闲端口；
  全占着才退到 OS 临时端口（`bind :0`）——于是自动模式**不会**因为端口被占而启动失败。
  只吞 `AddrInUse`，其它错误（地址不合法等）原样上报，免得被藏成「扫了一百个都不行」。
- `[x]` **`PLANET_X_WEB_PORT` 的三种语义**（`web/src/main.rs::parse_port_choice`）：
  * **不设 / `auto`** = 自动：从 `3000` 起扫 100 个 → 印出**实际**端口 + 一行
    「`3000` 已被占用 → 自动改用 3001」；`3000` 空着时行为与从前**逐字一致**。
  * **数字** = 点名：被占就直接报错，**不会**被悄悄换掉（显式端口静默漂移比启动失败更难查）。
    `0` = 让 OS 挑临时端口。
  * **别的字符串** = 明确报错（原文照抄：``PLANET_X_WEB_PORT=`30oo` 不是端口号（1-65535，或 `auto`）``），
    不再是 `127.0.0.1:30oo` 那种底层 DNS 报错。
- `[x]` 顺带印一行机器可读的 `PLANET_X_WEB_URL=http://127.0.0.1:<port>`，脚本/agent 直接抠，
  不用去解析那句中文。
- 验证：`cargo test -p planet_x_web` 5 passed（新增 `bind_auto_keeps_the_base_port_when_it_is_free` /
  `bind_auto_skips_a_busy_port` / `port_env_parsing`）；实机三连开：3000（空着，原样）→ 3001（3000 被占，
  印出漂移说明）→ 3002（`PLANET_X_WEB_PORT=auto`），三个都 `GET /api/state` 200；
  显式 `PLANET_X_WEB_PORT=3000`（被占）→ 退出码 1 + 指名端口的报错。
- `[x]` **（已被 `web-lifecycle.md` 做掉）** 端口与身份回显：`GET /api/ping` 给 pid / 端口 / 二进制 + 构建时长
  （见 `web-lifecycle.md`）。「多实例互指/链接分享」仍没做，但前端现在能问出自己在哪个实例上了。
- `[ ]` **（留给以后）** `PLANET_X_WEB_HOST` 覆盖监听地址（现在硬编 `127.0.0.1`；要局域网/手机看就得放开，
  但那属于**安全**决定，别顺手做）。
