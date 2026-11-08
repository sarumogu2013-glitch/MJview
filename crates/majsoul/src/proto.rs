//! 雀魂の通信（protobuf）を、ゲーム自身の型定義（liqi.json。protobuf.js の JSON 形式）を使って JSON にデコードする。
//!
//! 型定義はゲームが起動時に読み込むものをそのまま使うので、雀魂の更新で項目が増えても追従できる。
//! 知らない項目は飛ばす。bytes は base64 の文字列にする。

use std::collections::HashMap;

use base64::Engine;
use serde_json::{Map, Value, json};

#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub id: u32,
    pub ty: String,
    pub repeated: bool,
}

#[derive(Debug)]
pub enum TypeDef {
    Message(Vec<Field>),
    Enum,
}

#[derive(Debug, Clone)]
pub struct Method {
    pub request: String,
    pub response: String,
}

/// 型定義。名前は先頭の . を除いた完全な名前（例: `lq.RecordNewRound`）
#[derive(Debug, Default)]
pub struct Schema {
    pub types: HashMap<String, TypeDef>,
    /// `lq.Lobby.fetchGameRecord` → 要求・応答の型（完全な名前）
    pub methods: HashMap<String, Method>,
}

impl Schema {
    /// protobuf.js の JSON（`{"nested": {...}}`）を読む
    pub fn from_json(text: &str) -> Result<Schema, String> {
        let root: Value =
            serde_json::from_str(text).map_err(|e| format!("型定義を読めません: {e}"))?;
        let mut s = Schema::default();
        let mut raw_fields: Vec<(String, String, Vec<(String, u32, String, bool)>)> = vec![];
        let mut raw_methods: Vec<(String, String, String, String)> = vec![];
        fn walk(
            v: &Value,
            prefix: &str,
            s: &mut Schema,
            raw_fields: &mut Vec<(String, String, Vec<(String, u32, String, bool)>)>,
            raw_methods: &mut Vec<(String, String, String, String)>,
        ) {
            let Some(nested) = v.get("nested").and_then(Value::as_object) else {
                return;
            };
            for (name, def) in nested {
                let full = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                if let Some(fields) = def.get("fields").and_then(Value::as_object) {
                    let fs = fields
                        .iter()
                        .filter_map(|(fname, f)| {
                            Some((
                                fname.clone(),
                                f.get("id")?.as_u64()? as u32,
                                f.get("type")?.as_str()?.to_string(),
                                f.get("rule").and_then(Value::as_str) == Some("repeated"),
                            ))
                        })
                        .collect();
                    raw_fields.push((full.clone(), prefix.to_string(), fs));
                } else if def.get("values").is_some() {
                    s.types.insert(full.clone(), TypeDef::Enum);
                }
                if let Some(methods) = def.get("methods").and_then(Value::as_object) {
                    for (mname, m) in methods {
                        if let (Some(req), Some(res)) = (
                            m.get("requestType").and_then(Value::as_str),
                            m.get("responseType").and_then(Value::as_str),
                        ) {
                            raw_methods.push((
                                format!("{full}.{mname}"),
                                full.clone(),
                                req.into(),
                                res.into(),
                            ));
                        }
                    }
                }
                walk(def, &full, s, raw_fields, raw_methods);
            }
        }
        walk(&root, "", &mut s, &mut raw_fields, &mut raw_methods);
        // 型の名前を完全な名前に直す（入れ子の型・別の名前空間の型）
        let names: Vec<String> = raw_fields.iter().map(|f| f.0.clone()).collect();
        for (full, _, fs) in &raw_fields {
            s.types.insert(full.clone(), TypeDef::Message(vec![]));
            let _ = fs;
        }
        let _ = names;
        let resolved: Vec<(String, Vec<Field>)> = raw_fields
            .iter()
            .map(|(full, _scope, fs)| {
                let fields = fs
                    .iter()
                    .map(|(name, id, ty, repeated)| Field {
                        name: name.clone(),
                        id: *id,
                        ty: s.resolve(ty, full),
                        repeated: *repeated,
                    })
                    .collect();
                (full.clone(), fields)
            })
            .collect();
        for (full, fields) in resolved {
            s.types.insert(full, TypeDef::Message(fields));
        }
        for (full, scope, req, res) in raw_methods {
            let m = Method {
                request: s.resolve(&req, &scope),
                response: s.resolve(&res, &scope),
            };
            s.methods.insert(full, m);
        }
        if s.types.is_empty() {
            return Err("型定義が空です".into());
        }
        Ok(s)
    }

    /// 型の名前を完全な名前にする。スカラー型はそのまま
    fn resolve(&self, ty: &str, scope: &str) -> String {
        if is_scalar(ty) {
            return ty.to_string();
        }
        let ty = ty.trim_start_matches('.');
        // scope（自分の型の完全な名前）から外へ向かって探す
        let mut sc = scope.to_string();
        loop {
            let cand = if sc.is_empty() {
                ty.to_string()
            } else {
                format!("{sc}.{ty}")
            };
            if self.types.contains_key(&cand) {
                return cand;
            }
            match sc.rfind('.') {
                Some(i) => sc.truncate(i),
                None if !sc.is_empty() => sc.clear(),
                None => break,
            }
        }
        ty.to_string()
    }

    /// `name`（完全な名前。先頭の . はあってもよい）の型として `data` をデコードする
    pub fn decode(&self, name: &str, data: &[u8]) -> Result<Value, String> {
        let name = name.trim_start_matches('.');
        let Some(TypeDef::Message(fields)) = self.types.get(name) else {
            return Err(format!("型 {name} がありません"));
        };
        let mut out = Map::new();
        let mut r = Reader { b: data, p: 0 };
        while r.p < r.b.len() {
            let key = r.varint()?;
            let (id, wt) = ((key >> 3) as u32, (key & 7) as u8);
            let Some(f) = fields.iter().find(|f| f.id == id) else {
                r.skip(wt)?;
                continue;
            };
            let push = |out: &mut Map<String, Value>, v: Value| {
                if f.repeated {
                    match out.entry(f.name.clone()).or_insert_with(|| json!([])) {
                        Value::Array(a) => a.push(v),
                        other => *other = json!([v]),
                    }
                } else {
                    out.insert(f.name.clone(), v);
                }
            };
            if wt == 2 && is_packable(&f.ty) {
                // packed（数値の繰り返しをまとめたもの）
                let bytes = r.bytes()?;
                let mut pr = Reader { b: bytes, p: 0 };
                while pr.p < pr.b.len() {
                    let v = pr.scalar(&f.ty, wire_of(&f.ty))?;
                    push(&mut out, v);
                }
                continue;
            }
            let v = if is_scalar(&f.ty) {
                r.scalar(&f.ty, wt)?
            } else {
                match self.types.get(&f.ty) {
                    Some(TypeDef::Enum) => json!(r.varint()? as i64),
                    Some(TypeDef::Message(_)) => {
                        let b = r.bytes()?;
                        self.decode(&f.ty, b)?
                    }
                    None => {
                        r.skip(wt)?;
                        continue;
                    }
                }
            };
            push(&mut out, v);
        }
        Ok(Value::Object(out))
    }
}

fn is_scalar(t: &str) -> bool {
    matches!(
        t,
        "double"
            | "float"
            | "int32"
            | "int64"
            | "uint32"
            | "uint64"
            | "sint32"
            | "sint64"
            | "fixed32"
            | "fixed64"
            | "sfixed32"
            | "sfixed64"
            | "bool"
            | "string"
            | "bytes"
    )
}

fn is_packable(t: &str) -> bool {
    is_scalar(t) && t != "string" && t != "bytes"
}

fn wire_of(t: &str) -> u8 {
    match t {
        "double" | "fixed64" | "sfixed64" => 1,
        "float" | "fixed32" | "sfixed32" => 5,
        "string" | "bytes" => 2,
        _ => 0,
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.p).ok_or("データが途中で切れています")?;
            self.p += 1;
            v |= ((byte & 0x7F) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("数値が長すぎます".into())
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .p
            .checked_add(n)
            .filter(|e| *e <= self.b.len())
            .ok_or("データが途中で切れています")?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.varint()? as usize;
        self.take(n)
    }
    fn skip(&mut self, wt: u8) -> Result<(), String> {
        match wt {
            0 => {
                self.varint()?;
            }
            1 => {
                self.take(8)?;
            }
            2 => {
                self.bytes()?;
            }
            5 => {
                self.take(4)?;
            }
            _ => return Err(format!("知らない形式 {wt}")),
        }
        Ok(())
    }
    fn scalar(&mut self, ty: &str, _wt: u8) -> Result<Value, String> {
        Ok(match ty {
            "string" => json!(String::from_utf8_lossy(self.bytes()?)),
            "bytes" => json!(base64::engine::general_purpose::STANDARD.encode(self.bytes()?)),
            "bool" => json!(self.varint()? != 0),
            "int32" | "int64" => json!(self.varint()? as i64),
            "uint32" | "uint64" => json!(self.varint()?),
            "sint32" | "sint64" => {
                let v = self.varint()?;
                json!(((v >> 1) as i64) ^ -((v & 1) as i64))
            }
            "fixed32" => json!(u32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            "sfixed32" => json!(i32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            "float" => json!(f32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            "fixed64" => json!(u64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            "sfixed64" => json!(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            "double" => json!(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            _ => return Err(format!("知らない型 {ty}")),
        })
    }
}

/// base64 の bytes を戻す
pub fn unb64(v: &Value) -> Vec<u8> {
    v.as_str()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .unwrap_or_default()
}

/// 小さな protobuf の書き手（テストと、偽の雀魂ページを作る確認用）
pub mod enc {
    pub fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
    pub fn key(out: &mut Vec<u8>, id: u32, wt: u8) {
        varint(out, ((id as u64) << 3) | wt as u64);
    }
    pub fn uint(out: &mut Vec<u8>, id: u32, v: u64) {
        key(out, id, 0);
        varint(out, v);
    }
    pub fn bytes(out: &mut Vec<u8>, id: u32, b: &[u8]) {
        key(out, id, 2);
        varint(out, b.len() as u64);
        out.extend_from_slice(b);
    }
    pub fn string(out: &mut Vec<u8>, id: u32, s: &str) {
        bytes(out, id, s.as_bytes());
    }
    pub fn packed(out: &mut Vec<u8>, id: u32, vs: &[u64]) {
        let mut b = vec![];
        for v in vs {
            varint(&mut b, *v);
        }
        bytes(out, id, &b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"{"nested":{"lq":{"nested":{
        "Wrapper":{"fields":{"name":{"type":"string","id":1},"data":{"type":"bytes","id":2}}},
        "Inner":{"fields":{"tile":{"type":"string","id":1},"seat":{"type":"uint32","id":2}}},
        "Outer":{"fields":{
            "scores":{"rule":"repeated","type":"int32","id":1},
            "inner":{"type":"Inner","id":2},
            "list":{"rule":"repeated","type":"Inner","id":3},
            "flag":{"type":"bool","id":4},
            "kind":{"type":"Kind","id":5},
            "names":{"rule":"repeated","type":"string","id":6},
            "neg":{"type":"sint32","id":7}
        },"nested":{"Kind":{"values":{"A":0,"B":1}}}},
        "Lobby":{"methods":{"fetchGameRecord":{"requestType":"Outer","responseType":"Inner"}}}
    }}}}"#;

    #[test]
    fn decodes_nested_repeated_packed_enum() {
        let s = Schema::from_json(SCHEMA).unwrap();
        assert_eq!(s.methods["lq.Lobby.fetchGameRecord"].response, "lq.Inner");
        let mut inner = vec![];
        enc::string(&mut inner, 1, "5p");
        enc::uint(&mut inner, 2, 3);
        let mut b = vec![];
        enc::packed(&mut b, 1, &[25000, 30000]);
        enc::bytes(&mut b, 2, &inner);
        enc::bytes(&mut b, 3, &inner);
        enc::bytes(&mut b, 3, &inner);
        enc::uint(&mut b, 4, 1);
        enc::uint(&mut b, 5, 1);
        enc::string(&mut b, 6, "選手");
        enc::uint(&mut b, 7, 3); // sint32: -2
        enc::uint(&mut b, 99, 7); // 知らない項目は飛ばす
        let v = s.decode(".lq.Outer", &b).unwrap();
        assert_eq!(v["scores"], json!([25000, 30000]));
        assert_eq!(v["inner"]["tile"], "5p");
        assert_eq!(v["list"].as_array().unwrap().len(), 2);
        assert_eq!(v["flag"], true);
        assert_eq!(v["kind"], 1);
        assert_eq!(v["names"], json!(["選手"]));
        assert_eq!(v["neg"], -2);
    }

    #[test]
    fn broken_data_is_an_error() {
        let s = Schema::from_json(SCHEMA).unwrap();
        assert!(s.decode("lq.Inner", &[0x0A, 0x05, b'a']).is_err());
        assert!(s.decode("lq.Nope", &[]).is_err());
    }
}
