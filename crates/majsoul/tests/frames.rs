//! 雀魂のフレーム（要求の番号と応答の対応）と、牌譜の bytes から記録を取り出すところ。

use janscope_majsoul::convert::records;
use janscope_majsoul::frames::Tracker;
use janscope_majsoul::proto::{Schema, enc};

const SCHEMA: &str = r#"{"nested":{"lq":{"nested":{
 "Wrapper":{"fields":{"name":{"type":"string","id":1},"data":{"type":"bytes","id":2}}},
 "Lobby":{"methods":{"fetchGameRecord":{"requestType":"ReqGameRecord","responseType":"ResGameRecord"}}},
 "ReqGameRecord":{"fields":{"game_uuid":{"type":"string","id":1}}},
 "ResGameRecord":{"fields":{"data":{"type":"bytes","id":4}}},
 "GameDetailRecords":{"fields":{"records":{"rule":"repeated","type":"bytes","id":1},"actions":{"rule":"repeated","type":"GameAction","id":3}}},
 "GameAction":{"fields":{"result":{"type":"bytes","id":3}}},
 "RecordDealTile":{"fields":{"seat":{"type":"uint32","id":1},"left_tile_count":{"type":"uint32","id":3}}}
}}}}"#;

fn wrap(name: &str, data: &[u8]) -> Vec<u8> {
    let mut w = vec![];
    enc::string(&mut w, 1, name);
    enc::bytes(&mut w, 2, data);
    w
}

#[test]
fn response_is_matched_to_request_by_index() {
    let s = Schema::from_json(SCHEMA).unwrap();
    let mut t = Tracker::default();
    // 要求 #7（牌譜）と #8（別のもの）
    let mut req = vec![2, 7, 0];
    let mut body = vec![];
    enc::string(&mut body, 1, "uuid");
    req.extend(wrap(".lq.Lobby.fetchGameRecord", &body));
    t.on_sent(&s, &req);
    // 応答（名前なし）
    let mut deal = vec![];
    enc::uint(&mut deal, 1, 2);
    enc::uint(&mut deal, 3, 55);
    let mut action = vec![];
    enc::bytes(&mut action, 3, &wrap(".lq.RecordDealTile", &deal));
    let mut detail = vec![];
    enc::bytes(&mut detail, 3, &action);
    let data = wrap(".lq.GameDetailRecords", &detail);
    let mut res = vec![];
    enc::bytes(&mut res, 4, &data);
    let mut frame = vec![3, 7, 0];
    frame.extend(wrap("", &res));
    let m = t.on_received(&s, &frame).expect("応答が対応する");
    assert_eq!(m.name, "lq.Lobby.fetchGameRecord");
    // 中身から記録を取り出す
    let bytes = janscope_majsoul::proto::unb64(&m.body["data"]);
    let recs = records(&s, &bytes).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].name, "lq.RecordDealTile");
    assert_eq!(recs[0].body["left_tile_count"], 55);
    // 同じ番号の応答は1回だけ
    assert!(t.on_received(&s, &frame).is_none());
}
