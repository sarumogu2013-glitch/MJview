//! 操作ログ・Undo/Redo・保存と復元（設計書 3.1 イベントソーシング、5.5）。
//!
//! 状態は「有効な操作の列」を先頭から適用して作る。ファイルには操作・Undo・Redo を JSON Lines で
//! 追記するので、途中で落ちても読み直せば Redo の候補まで含めて同じ状態に戻る。

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::engine::{EngineError, Overlay, apply};
use crate::state::{Action, TableState};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stamped {
    /// サーバー時刻（UNIX ミリ秒）
    pub at: i64,
    pub action: Action,
}

/// ログファイルの1行。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum LogOp {
    Do(Stamped),
    Undo { at: i64 },
    Redo { at: i64 },
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("ログファイルの読み書きに失敗しました: {0}")]
    Io(#[from] std::io::Error),
    #[error("ログファイルの {line} 行目が読めません: {msg}")]
    Parse { line: usize, msg: String },
    #[error("取り消せる操作がありません")]
    NothingToUndo,
    #[error("やり直せる操作がありません")]
    NothingToRedo,
}

/// 操作ログと現在の状態。
#[derive(Debug, Default)]
pub struct GameLog {
    done: Vec<Stamped>,
    undone: Vec<Stamped>,
    state: TableState,
    file: Option<(PathBuf, File)>,
}

impl GameLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// 保存先のファイルを作って記録を始める（既存のファイルには追記しない。復元は `open`）。
    pub fn create(path: &Path) -> Result<Self, LogError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let f = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(path)?;
        Ok(GameLog {
            file: Some((path.to_path_buf(), f)),
            ..Self::default()
        })
    }

    /// 既存のログファイルを読み込み、続きを追記する（クラッシュ後の「続きから再開」）。
    pub fn open(path: &Path) -> Result<Self, LogError> {
        let mut log = GameLog::new();
        let reader = BufReader::new(File::open(path)?);
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let op: LogOp = match serde_json::from_str(&line) {
                Ok(op) => op,
                // 書き込み途中で落ちた最後の行は捨てる
                Err(e) if e.is_eof() => break,
                Err(e) => {
                    return Err(LogError::Parse {
                        line: i + 1,
                        msg: e.to_string(),
                    });
                }
            };
            log.replay_op(op)?;
        }
        let f = OpenOptions::new().append(true).open(path)?;
        log.file = Some((path.to_path_buf(), f));
        Ok(log)
    }

    fn replay_op(&mut self, op: LogOp) -> Result<(), LogError> {
        match op {
            LogOp::Do(st) => {
                apply(&mut self.state, &st.action)?;
                self.done.push(st);
                self.undone.clear();
            }
            LogOp::Undo { .. } => {
                let st = self.done.pop().ok_or(LogError::NothingToUndo)?;
                self.undone.push(st);
                self.rebuild();
            }
            LogOp::Redo { .. } => {
                let st = self.undone.pop().ok_or(LogError::NothingToRedo)?;
                apply(&mut self.state, &st.action)?;
                self.done.push(st);
            }
        }
        Ok(())
    }

    fn write(&mut self, op: &LogOp) -> Result<(), LogError> {
        if let Some((_, f)) = &mut self.file {
            let mut line = serde_json::to_string(op).expect("serialize");
            line.push('\n');
            f.write_all(line.as_bytes())?;
            f.flush()?;
        }
        Ok(())
    }

    fn rebuild(&mut self) {
        let mut st = TableState::default();
        for a in &self.done {
            // 記録済みの操作は一度成功しているので、同じ順に適用すれば必ず成功する
            apply(&mut st, &a.action).expect("ログの再生に失敗しました");
        }
        self.state = st;
    }

    pub fn state(&self) -> &TableState {
        &self.state
    }
    pub fn path(&self) -> Option<&Path> {
        self.file.as_ref().map(|(p, _)| p.as_path())
    }
    pub fn actions(&self) -> &[Stamped] {
        &self.done
    }
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// 操作を適用して記録する。失敗したら何も変わらない。
    pub fn act(&mut self, action: Action, at: i64) -> Result<Vec<Overlay>, LogError> {
        let ov = apply(&mut self.state, &action)?;
        let st = Stamped { at, action };
        self.write(&LogOp::Do(st.clone()))?;
        self.done.push(st);
        self.undone.clear();
        Ok(ov)
    }

    pub fn undo(&mut self, at: i64) -> Result<(), LogError> {
        if self.done.is_empty() {
            return Err(LogError::NothingToUndo);
        }
        self.write(&LogOp::Undo { at })?;
        let st = self.done.pop().unwrap();
        self.undone.push(st);
        self.rebuild();
        Ok(())
    }

    pub fn redo(&mut self, at: i64) -> Result<Vec<Overlay>, LogError> {
        let Some(st) = self.undone.last().cloned() else {
            return Err(LogError::NothingToRedo);
        };
        let ov = apply(&mut self.state, &st.action)?;
        self.write(&LogOp::Redo { at })?;
        self.undone.pop();
        self.done.push(st);
        Ok(ov)
    }
}
