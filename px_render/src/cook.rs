use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use crate::edit::ParamStore;

#[derive(Debug, Clone)]
pub enum Progress {
    Step {
        index: usize,
        total: usize,
        what: String,
    },
    Line(String),
    Done {
        scene: Option<(PathBuf, u64)>,
        millis: u128,
    },
    Failed {
        what: String,
        code: i32,
        millis: u128,
    },
}

pub struct Cook {
    rx: Option<Receiver<Progress>>,
    current: Option<String>,
    total: usize,
    done: usize,
    lines: Vec<String>,
    failed: Option<String>,
    started: Option<std::time::Instant>,
}

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

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

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

    pub fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn tail(&self, count: usize) -> impl Iterator<Item = &str> {
        let skip = self.lines.len().saturating_sub(count);
        self.lines[skip..].iter().map(String::as_str)
    }

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

#[derive(Debug, Clone)]
pub struct SceneUpdate {
    pub scene: Option<(PathBuf, u64)>,
}

pub fn start(store: &ParamStore, recipe: &str, pcg_root: &Path) -> Result<Cook, String> {
    let px = px_exe(store.root())?;
    let store_root = store.store_root().to_path_buf();
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

fn steps_len(store: &ParamStore) -> usize {
    store.cook_order().len() + 1
}

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

fn scene_from_manifest(pcg_root: &Path) -> Option<(PathBuf, u64)> {
    let path = pcg_root.join("scene").join("manifest.json");
    let text = std::fs::read_to_string(&path).ok()?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    let key = entries.last()?.get("key")?.as_str()?;
    let bytes = px_protocol::scene::cas_path(pcg_root, key).ok()?;
    let fingerprint = fingerprint_of(&bytes)?;
    Some((bytes, fingerprint))
}

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
