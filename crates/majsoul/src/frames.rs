//! 雀魂の WebSocket のフレーム。先頭1バイトが種類（1=通知・2=要求・3=応答）、要求と応答は続く2バイトが番号（リトルエンディアン）、
//! そのあとが `Wrapper { name, data }`（protobuf）。応答には名前が入らないので、同じ番号の要求の名前から応答の型を決める。

use std::collections::HashMap;

use serde_json::Value;

use crate::proto::{Schema, unb64};

/// 受け取ったものの中身
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// 要求の名前（例: `lq.Lobby.fetchGameRecord`）。通知なら通知の型の名前
    pub name: String,
    pub body: Value,
}

#[derive(Default)]
pub struct Tracker {
    pending: HashMap<u16, String>,
}

fn wrapper(schema: &Schema, b: &[u8]) -> Option<(String, Vec<u8>)> {
    let w = schema.decode("lq.Wrapper", b).ok()?;
    let name = w
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim_start_matches('.')
        .to_string();
    let data = w.get("data").map(unb64).unwrap_or_default();
    Some((name, data))
}

impl Tracker {
    /// ブラウザが送ったフレーム（要求）を覚える
    pub fn on_sent(&mut self, schema: &Schema, frame: &[u8]) {
        if frame.len() < 3 || frame[0] != 2 {
            return;
        }
        let idx = u16::from_le_bytes([frame[1], frame[2]]);
        if let Some((name, _)) = wrapper(schema, &frame[3..]) {
            self.pending.insert(idx, name);
            // 古いものは捨てる（応答が来なかった要求）
            if self.pending.len() > 256 {
                let keys: Vec<u16> = self.pending.keys().copied().take(64).collect();
                for k in keys {
                    self.pending.remove(&k);
                }
            }
        }
    }

    /// ブラウザが受け取ったフレーム。要求への応答か通知なら中身を返す
    pub fn on_received(&mut self, schema: &Schema, frame: &[u8]) -> Option<Message> {
        match *frame.first()? {
            1 => {
                let (name, data) = wrapper(schema, &frame[1..])?;
                let body = schema.decode(&name, &data).ok()?;
                Some(Message { name, body })
            }
            3 if frame.len() >= 3 => {
                let idx = u16::from_le_bytes([frame[1], frame[2]]);
                let method = self.pending.remove(&idx)?;
                let res = schema.methods.get(&method)?.response.clone();
                let (_, data) = wrapper(schema, &frame[3..])?;
                let body = schema.decode(&res, &data).ok()?;
                Some(Message { name: method, body })
            }
            _ => None,
        }
    }
}
