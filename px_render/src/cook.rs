//! **烘图那一半**（S9）：把 [`crate::edit::ParamStore`] 的会话副本交给 `px run` 去烘。
//!
//! 它做的只有四件事，每件都有理由：
//!
//! 1. **起子进程，不在本进程算**。`px_render` 只读产物（`art.rs` 顶上那条），算节点的是
//!    图程序；而"哪条命令能烘这张图"已经有一个答案（`px run`），这里不造第二个。
//! 2. **在一条后台线程上跑**。一次全量烘是**分钟**量级（`nebula --face 256` 那种），
//!    窗口的事件循环一秒都不能被它挡住 —— 挡住的话拖一下鼠标要等烘完才动。
//! 3. **输出原样收下来**（stdout + stderr，按行），不解析、不改写：判据与"我该看哪一行"
//!    的答案在那些行里（`px` 自己的审计口径），这里只负责把它们端到面板上。
//! 4. **烘完把"场景产物现在是哪一份"回报**。回报的是**盘上的事实**（重读
//!    `target/pcg/scene/manifest.json`），不是"我以为它写到哪儿" —— 键是算出来的，
//!    路径是拼出来的，两处各猜一遍就有一处会错。
//!
//! ## 顺序是硬的
//!
//! ```text
//! px run <图1> --store <副本>   ← 被引用的图先（nebulasky 吃 nebula 的产物）
//! px run <图2> --store <副本>
//! px run scene <配方> --store <副本>
//! ```
//!
//! ⚠ 最后那一步是**场景**：面板改的是图参数，而窗口显示的是**场景产物**（`.pxart`）——
//!   不重烘场景，改完图参数之后画面什么都不会发生（那是本单元最容易"看着像没生效"的一步）。
//! ⚠ 图与场景的参数**不在同一张图里**：`scene` 那张图的参数目录是 `art/scene/`，
//!   它不在会话副本里 —— 所以场景那一步**不带 `--store`**（带了就会去副本里找一份
//!   不存在的 `art/scene/`，`node_params` 缺文件是**静默走 Default** 的，症状是
//!   "改完图参数之后场景参数全变默认值"）。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use crate::edit::ParamStore;

/// 一次烘图的进度（面板按这个更新那一行状态）。
#[derive(Debug, Clone)]
pub enum Progress {
    /// 正在跑哪一步（`px run <图>`）。
    Step {
        /// 第几步、共几步（1 起）。
        index: usize,
        total: usize,
        /// 这一步烘的是什么（`nebula` / `scene nebula`）。
        what: String,
    },
    /// 一行输出（**原样**：`px` 与图程序的审计行）。
    Line(String),
    /// 烘完了。`scene` 是**重读清单拿到的**那一份（路径 + 内容键）。
    Done {
        /// 烘好的场景产物。`None` = 清单读不到（那就别换画面，把理由说清楚）。
        scene: Option<(PathBuf, u64)>,
        /// 全程墙钟毫秒。
        millis: u128,
    },
    /// 哪一步非零退出（后面就不跑了）。
    Failed {
        what: String,
        code: i32,
        millis: u128,
    },
}

/// 面板手上那一份"正在烘 / 刚烘完"的状态。
pub struct Cook {
    rx: Option<Receiver<Progress>>,
    /// 现在这一步是什么（面板拿它当状态行的主词）。
    current: Option<String>,
    total: usize,
    done: usize,
    lines: Vec<String>,
    /// 收起来的那些行（面板默认只显示尾巴；**全留**是为了"翻回去看第一行报的什么"）。
    failed: Option<String>,
    started: Option<std::time::Instant>,
}

/// 面板最多留多少行输出（够看一次烘图的尾巴，又不会把内存吃穿）。
const KEEP_LINES: usize = 400;

impl Default for Cook {
    fn default() -> Self {
        Cook::new()
    }
}

impl Cook {
    pub fn new() -> Cook {
        Cook {
            rx: None,
            current: None,
            total: 0,
            done: 0,
            lines: Vec::new(),
            failed: None,
            started: None,
        }
    }

    /// 正在烘吗。
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// 状态行：现在到哪一步了。
    pub fn status(&self) -> String {
        match (&self.current, self.rx.is_some()) {
            (Some(what), true) => format!(
                "烘图中 {}/{}：{what}｜{:.1} s",
                self.done + 1,
                self.total,
                self.started
                    .map(|started| started.elapsed().as_secs_f32())
                    .unwrap_or(0.0)
            ),
            (Some(what), false) => format!("上一次：{what}"),
            (None, _) => "还没烘过".to_string(),
        }
    }

    /// 失败那一句话（没有就是 `None`）。
    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    /// 最近那几行输出（面板显示"烘图在说什么"）。
    pub fn tail(&self, count: usize) -> impl Iterator<Item = &str> {
        let skip = self.lines.len().saturating_sub(count);
        self.lines[skip..].iter().map(String::as_str)
    }

    /// 每一 tick 收一次进度。交回"这一次收到了 [`Progress::Done`]"。
    ///
    /// ⚠ 它**不阻塞**：窗口的 tick 是 200 ms 一次，等在这里就等于把帧循环交给子进程。
    pub fn pump(&mut self) -> Option<SceneUpdate> {
        let mut update = None;
        loop {
            let Some(rx) = self.rx.as_ref() else {
                break;
            };
            match rx.try_recv() {
                Ok(progress) => {
                    if let Progress::Done { scene, .. } = &progress {
                        update = Some(SceneUpdate {
                            scene: scene.clone(),
                        });
                    }
                    self.apply(progress);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    break;
                }
            }
        }
        update
    }

    fn apply(&mut self, progress: Progress) {
        match progress {
            Progress::Step { index, total, what } => {
                self.done = index.saturating_sub(1);
                self.total = total;
                self.current = Some(what.clone());
                self.push(format!("── 第 {index}/{total} 步：{what}"));
            }
            Progress::Line(line) => self.push(line),
            Progress::Done { scene, millis } => {
                self.rx = None;
                self.done = self.total;
                self.started = None;
                match &scene {
                    Some((path, key)) => self.push(format!(
                        "── 烘完（{:.1} s）⇒ 场景 {}（键 {key:016x}）",
                        millis as f64 / 1000.0,
                        path.display()
                    )),
                    None => self.push(format!(
                        "── 烘完（{:.1} s），但场景清单读不到 ⇒ 画面不换（上面那些行里有理由）",
                        millis as f64 / 1000.0
                    )),
                }
            }
            Progress::Failed { what, code, millis } => {
                self.rx = None;
                self.started = None;
                self.failed = Some(format!("{what} 退出码 {code}"));
                self.push(format!(
                    "── 失败：{what} 退出码 {code}（{:.1} s）",
                    millis as f64 / 1000.0
                ));
            }
        }
    }

    fn push(&mut self, line: String) {
        if self.lines.len() >= KEEP_LINES {
            self.lines.remove(0);
        }
        self.lines.push(line);
    }
}

/// 一次烘图交回的东西：**场景产物换成了哪一份**。
#[derive(Debug, Clone)]
pub struct SceneUpdate {
    pub scene: Option<(PathBuf, u64)>,
}

/// **起一次烘图**（立刻返回，活干在后台线程上）。
///
/// `pcg_root` 是**窗口正在读的那个 CAS 根**：清单要从那儿重读 —— 读别处的话，
/// 面板报的键与窗口载入的那一份可以不是同一个（那就是"看着像生效了、其实没有"）。
pub fn start(store: &ParamStore, recipe: &str, pcg_root: &Path) -> Result<Cook, String> {
    let px = px_exe(store.root())?;
    let store_root = store.store_root().to_path_buf();
    // 步骤表在**起线程之前**排好：它是"这一次要烘什么"的完整答案，
    // 排错在那里就能看出来，不必等子进程跑起来。
    let mut steps: Vec<Vec<String>> = Vec::new();
    for graph in store.cook_order() {
        steps.push(vec![
            "run".to_string(),
            graph.clone(),
            "--store".to_string(),
            store_root.display().to_string(),
        ]);
    }
    steps.push(vec![
        "run".to_string(),
        "scene".to_string(),
        recipe.to_string(),
    ]);

    let (tx, rx) = channel();
    let root = store.root().to_path_buf();
    let pcg_root = pcg_root.to_path_buf();
    let thread = std::thread::Builder::new()
        .name("px-cook".to_string())
        .spawn(move || drive(px, root, steps, pcg_root, tx))
        .map_err(|err| format!("起不了烘图那条线程：{err}"))?;
    // ⚠ 线程句柄**丢掉**（detach）：窗口关掉的那一刻不该等一次烘图跑完 ——
    //   子进程是它自己的进程，窗口退出不杀它（`px` 会跑完并把产物落盘，
    //   下一次开窗口就命中）。这与"不留陈租约"是同一条：进程死了，产物仍然是真的。
    drop(thread);

    Ok(Cook {
        rx: Some(rx),
        current: None,
        total: steps_len(&store),
        done: 0,
        lines: Vec::new(),
        failed: None,
        started: Some(std::time::Instant::now()),
    })
}

/// 这一次一共几步（图数 + 场景那一步）。
fn steps_len(store: &ParamStore) -> usize {
    store.cook_order().len() + 1
}

/// 后台线程那一趟：逐步起子进程、把输出端回面板、最后重读清单。
fn drive(
    px: PathBuf,
    root: PathBuf,
    steps: Vec<Vec<String>>,
    pcg_root: PathBuf,
    tx: Sender<Progress>,
) {
    let started = std::time::Instant::now();
    let total = steps.len();
    for (index, args) in steps.iter().enumerate() {
        let what = args
            .iter()
            .skip(1)
            .take_while(|arg| arg.as_str() != "--store")
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        if tx
            .send(Progress::Step {
                index: index + 1,
                total,
                what: what.clone(),
            })
            .is_err()
        {
            return;
        }
        let mut command = Command::new(&px);
        command
            .args(args)
            // ⚠ 工作目录 = 工作区根：`px` 自己的路径是绝对的吗？不是 —— 它要找
            //   **旁边的**图 exe（`graph_exe` 按 `current_exe` 推），而 CAS / `art/`
            //   那一侧全部按 `CARGO_MANIFEST_DIR`（编译期）算 ⇒ 这一格只影响
            //   "子进程的相对路径从哪儿起"，钉成根目录是为了让面板的命令与人在
            //   仓库根手敲的那一条**逐字相同**。
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                let _ = tx.send(Progress::Failed {
                    what: format!("起不了 {}（{err}）", px.display()),
                    code: -1,
                    millis: started.elapsed().as_millis(),
                });
                return;
            }
        };
        // 两路输出**都要收**：`px` 的审计走 stdout，而图程序报错走 stderr
        // （"改参数改坏了"那一档只在 stderr 上有）。
        let (out, err) = (child.stdout.take(), child.stderr.take());
        let readers = [
            out.map(|stream| spawn_reader("out", stream, tx.clone())),
            err.map(|stream| spawn_reader("err", stream, tx.clone())),
        ];
        let status = child.wait();
        for reader in readers.into_iter().flatten() {
            let _ = reader.join();
        }
        let code = match status {
            Ok(status) => status.code().unwrap_or(-1),
            Err(err) => {
                let _ = tx.send(Progress::Failed {
                    what: format!("{what}（等不到退出：{err}）"),
                    code: -1,
                    millis: started.elapsed().as_millis(),
                });
                return;
            }
        };
        if code != 0 {
            let _ = tx.send(Progress::Failed {
                what,
                code,
                millis: started.elapsed().as_millis(),
            });
            return;
        }
    }

    let _ = tx.send(Progress::Done {
        scene: scene_from_manifest(&pcg_root),
        millis: started.elapsed().as_millis(),
    });
}

/// 一行一行读子进程的输出（`err` 那一路加个前缀：**哪一路说的**要看得见）。
fn spawn_reader<R: std::io::Read + Send + 'static>(
    which: &'static str,
    stream: R,
    tx: Sender<Progress>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let line = if which == "err" {
                format!("[stderr] {line}")
            } else {
                line
            };
            if tx.send(Progress::Line(line)).is_err() {
                return;
            }
        }
    })
}

/// **场景产物现在是哪一份**：重读 `target/pcg/scene/manifest.json`。
///
/// ⚠ 路径与键都从清单里**读**，不猜：键是 `scene_key` 算出来的十六进制，路径是
///   `cas_path` 拼的。这里再算一遍就等于把同一个东西写两遍 —— 而它们**一定会**漂开
///   （`scene.rs` 尾巴上那一段专门说过：清单里印的键不是文件字节的 sha256）。
/// ⚠ 清单里住着**好几份**场景（`orbit` / `nebula` / …）：取**最后写进去的那一份**
///   （`scene.rs` 收尾时 `retain` 掉同名、再 `push` 到尾巴上）。所以"刚烘的是谁"
///   这个问题由**写的人**回答，不由读的人猜。
fn scene_from_manifest(pcg_root: &Path) -> Option<(PathBuf, u64)> {
    let path = pcg_root.join("scene").join("manifest.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    let key = entries.last()?.get("key")?.as_str()?;
    let bytes = px_protocol::scene::cas_path(pcg_root, key).ok()?;
    let fingerprint = fingerprint_of(&bytes)?;
    Some((bytes, fingerprint))
}

/// 一份场景产物的**载荷指纹**（清单帧里那一格）。
///
/// ⚠ 与 `viewer::fingerprint_of` 读的是**同一格**（`read_manifest` 的第一份资产）：
///   两处必须是同一个数，否则"面板烘完推给窗口"会因为一个对不上而当成"换了内容"
///   （或者反过来，静默不动）。
fn fingerprint_of(path: &Path) -> Option<u64> {
    let bundle = px_protocol::art::read_manifest(path).ok()?;
    Some(
        bundle
            .assets
            .first()
            .map(|asset| asset.fingerprint)
            .unwrap_or(0),
    )
}

/// `px` 在哪：**本 exe 旁边**（同一个 `target/<profile>/`），兜底找 debug/release。
///
/// ⚠ 找不到就**当场报错、把该跑什么写清楚**：面板安静地什么都不做是最坏的一种
///   （人按了 Cook，面板说"烘完了"，而盘上什么都没变）。
fn px_exe(root: &Path) -> Result<PathBuf, String> {
    let suffix = std::env::consts::EXE_SUFFIX;
    if let Ok(here) = std::env::current_exe() {
        if let Some(dir) = here.parent() {
            let beside = dir.join(format!("px{suffix}"));
            if beside.is_file() {
                return Ok(beside);
            }
        }
    }
    for profile in ["debug", "release"] {
        let candidate = root
            .join("target")
            .join(profile)
            .join(format!("px{suffix}"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "找不到 `px`（找过本 exe 旁边与 {}/target/{{debug,release}}/）⇒ 先编它：\
         cargo build -p px_graphs --bin px",
        root.display()
    ))
}
