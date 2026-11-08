//! 通信のテスト（設計書 8章: 切断・再接続・サーバー再起動で状態が正しく復元される。
//! 選手用画面ロールに手牌などの情報が一切届かない — 通信内容を記録して確認）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use janscope_core::protocol::{ClientMsg, ClientRole, PROTOCOL_VERSION};
use janscope_core::rules::GameConfig;
use janscope_core::scoring::WinInput;
use janscope_core::state::{Action, Phase, Player};
use janscope_core::tile::{SeatId, parse_tiles};
use janscope_core::view::TableStateView;
use janscope_netclient::{ConnectConfig, NetClient, NetEvent};
use janscope_server::{ServerConfig, VERSION, start};
use tokio_tungstenite::tungstenite::Message;

const CODE: &str = "424242";
const T: Duration = Duration::from_secs(5);

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("janscope-net-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn cfg(dir: &PathBuf, port: u16, resume: Option<PathBuf>) -> ServerConfig {
    ServerConfig {
        bind: SocketAddr::from(([127, 0, 0, 1], port)),
        code: CODE.into(),
        data_dir: dir.clone(),
        resume,
        dummy: false,
    }
}

fn client(addr: SocketAddr, role: ClientRole, code: &str) -> NetClient {
    NetClient::connect(ConnectConfig {
        addr: addr.to_string(),
        code: code.into(),
        role,
        client_version: VERSION.into(),
    })
}

/// 条件に合う出来事が来るまで待つ。
fn wait_for(c: &NetClient, mut f: impl FnMut(&NetEvent) -> bool) -> NetEvent {
    let deadline = std::time::Instant::now() + T;
    while std::time::Instant::now() < deadline {
        if let Some(e) = c.recv_timeout(Duration::from_millis(100)) {
            if f(&e) {
                return e;
            }
        }
    }
    panic!("待っている出来事が来ませんでした");
}

fn wait_result(c: &NetClient, id: u64) -> Option<String> {
    match wait_for(
        c,
        |e| matches!(e, NetEvent::ActionResult { request_id, .. } if *request_id == id),
    ) {
        NetEvent::ActionResult { error, .. } => error,
        _ => unreachable!(),
    }
}

fn start_game() -> Action {
    Action::StartGame {
        config: GameConfig::preset_standard(),
        players: ["選手A", "選手B", "選手C", "選手D"].map(Player::new),
        chiicha: SeatId(0),
    }
}

fn full_state(e: &NetEvent) -> Option<&janscope_core::TableState> {
    match e {
        NetEvent::State { view, .. } => match view.as_ref() {
            TableStateView::Full(f) => Some(&f.state),
            _ => None,
        },
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn operate_and_broadcast() {
    let dir = tmp_dir("op");
    let srv = start(cfg(&dir, 0, None)).await.unwrap();
    let addr = srv.addr;
    tokio::task::spawn_blocking(move || {
        let mut op = client(addr, ClientRole::Broadcast, CODE);
        let input = client(addr, ClientRole::Input, CODE);
        wait_for(&op, |e| matches!(e, NetEvent::State { snapshot: true, .. }));
        wait_for(&input, |e| {
            matches!(e, NetEvent::State { snapshot: true, .. })
        });
        let id = op.send_action(start_game());
        assert_eq!(wait_result(&op, id), None);
        // もう一方のクライアントにも届く（誰が操作したかも分かる）
        let e = wait_for(&input, |e| {
            matches!(
                e,
                NetEvent::State {
                    snapshot: false,
                    ..
                }
            )
        });
        assert!(matches!(
            e,
            NetEvent::State {
                seq: 1,
                by: Some(ClientRole::Broadcast),
                ..
            }
        ));
        assert_eq!(full_state(&e).unwrap().phase, Phase::BetweenHands);
        // 不正な操作はエラーが返り、状態は変わらない
        let id = op.send_action(Action::Riichi { seat: SeatId(0) });
        assert!(wait_result(&op, id).unwrap().contains("立直"));
        // 点数の手動変更 → 即反映（段階B1の完了条件）
        let id = op.send_action(Action::ManualAdjust {
            deltas: [1000, -1000, 0, 0],
            reason: "テスト".into(),
        });
        assert_eq!(wait_result(&op, id), None);
        let e = wait_for(&input, |e| matches!(e, NetEvent::State { seq: 2, .. }));
        assert_eq!(full_state(&e).unwrap().seats[0].score, 26_000);
        // Undo
        let id = op.undo();
        assert_eq!(wait_result(&op, id), None);
        let e = wait_for(&input, |e| matches!(e, NetEvent::State { seq: 3, .. }));
        assert_eq!(full_state(&e).unwrap().seats[0].score, 25_000);
        // 時刻合わせ
        wait_for(&op, |e| matches!(e, NetEvent::Clock { .. }));
    })
    .await
    .unwrap();
    srv.stop();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_code_is_rejected() {
    let dir = tmp_dir("code");
    let srv = start(cfg(&dir, 0, None)).await.unwrap();
    let addr = srv.addr;
    tokio::task::spawn_blocking(move || {
        let c = client(addr, ClientRole::Broadcast, "000000");
        let e = wait_for(&c, |e| matches!(e, NetEvent::Rejected { .. }));
        assert!(matches!(e, NetEvent::Rejected { reason } if reason.contains("接続コード")));
    })
    .await
    .unwrap();
    srv.stop();
    std::fs::remove_dir_all(dir).ok();
}

/// 選手用画面ロールの通信内容をすべて記録し、手牌の情報が含まれないことを確かめる。
#[tokio::test(flavor = "multi_thread")]
async fn player_screen_never_receives_hand_tiles() {
    let dir = tmp_dir("ps");
    let srv = start(cfg(&dir, 0, None)).await.unwrap();
    let addr = srv.addr;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .unwrap();
    let join = ClientMsg::Join {
        role: ClientRole::PlayerScreen,
        code: CODE.into(),
        client_version: VERSION.into(),
        protocol: PROTOCOL_VERSION,
    };
    ws.send(Message::text(join.to_json())).await.unwrap();
    // 選手用画面からの操作は拒否される
    let act = ClientMsg::Action {
        request_id: 9,
        action: start_game(),
    };
    ws.send(Message::text(act.to_json())).await.unwrap();
    let preview = ClientMsg::PreviewRequest {
        request_id: 10,
        input: pinfu(),
    };
    ws.send(Message::text(preview.to_json())).await.unwrap();

    let recorder = tokio::spawn(async move {
        let mut all = Vec::new();
        while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            if let Message::Text(t) = m {
                all.push(t.to_string());
            }
        }
        all
    });

    tokio::task::spawn_blocking(move || {
        let mut op = client(addr, ClientRole::Broadcast, CODE);
        wait_for(&op, |e| matches!(e, NetEvent::State { snapshot: true, .. }));
        for a in [start_game(), Action::StartHand { dice: vec![(2, 5)] }] {
            let id = op.send_action(a);
            assert_eq!(wait_result(&op, id), None);
        }
        // プレビュー（配信側だけに返る）
        let pid = op.preview(pinfu());
        wait_for(
            &op,
            |e| matches!(e, NetEvent::Preview { request_id, result: Ok(_) } if *request_id == pid),
        );
        let id = op.send_action(Action::Win {
            wins: vec![pinfu()],
        });
        assert_eq!(wait_result(&op, id), None);
    })
    .await
    .unwrap();

    let all = recorder.await.unwrap();
    let joined = all.join("\n");
    assert!(
        all.iter().any(|m| m.contains("\"Public\"")),
        "公開情報は届いている"
    );
    assert!(joined.contains("平和"), "アガリの役は届いている");
    assert!(!joined.contains("\"Full\""), "全情報の状態が届いている");
    for bad in ["closed", "groups", "win_tile", "PreviewResult", "\"7m\""] {
        assert!(
            !joined.contains(bad),
            "選手用画面に {bad} が届いている:\n{joined}"
        );
    }
    assert!(joined.contains("このロールでは操作できません"));
    srv.stop();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_after_server_restart() {
    let dir = tmp_dir("restart");
    let srv = start(cfg(&dir, 0, None)).await.unwrap();
    let addr = srv.addr;
    let log_path = srv.log_path.clone().unwrap();
    let c = tokio::task::spawn_blocking(move || {
        let mut c = client(addr, ClientRole::Broadcast, CODE);
        wait_for(&c, |e| matches!(e, NetEvent::State { snapshot: true, .. }));
        let id = c.send_action(start_game());
        assert_eq!(wait_result(&c, id), None);
        let id = c.send_action(Action::ManualAdjust {
            deltas: [0, 0, 500, -500],
            reason: "復元テスト".into(),
        });
        assert_eq!(wait_result(&c, id), None);
        c
    })
    .await
    .unwrap();
    // サーバーが落ちる → 同じポートで、ログから再開
    srv.stop();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let srv2 = start(cfg(&dir, addr.port(), Some(log_path))).await.unwrap();
    tokio::task::spawn_blocking(move || {
        wait_for(&c, |e| matches!(e, NetEvent::Disconnected { .. }));
        let e = wait_for(&c, |e| matches!(e, NetEvent::State { snapshot: true, .. }));
        let st = full_state(&e).unwrap();
        assert_eq!(st.seats[2].score, 25_500);
        assert_eq!(st.phase, Phase::BetweenHands);
    })
    .await
    .unwrap();
    srv2.stop();
    std::fs::remove_dir_all(dir).ok();
}

fn pinfu() -> WinInput {
    WinInput {
        winner: SeatId(1),
        loser: Some(SeatId(2)),
        closed: parse_tiles("234567m345p678s99p").unwrap(),
        melds: vec![],
        win_tile: "2m".parse().unwrap(),
        riichi: false,
        double_riichi: false,
        ippatsu: false,
        haitei: false,
        rinshan: false,
        chankan: false,
        tenhou: false,
        ura_indicators: vec![],
        pao: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tournament_records_finished_games() {
    let dir = tmp_dir("tour");
    let srv = start(cfg(&dir, 0, None)).await.unwrap();
    let addr = srv.addr;
    let dir2 = dir.clone();
    tokio::task::spawn_blocking(move || {
        let mut c = client(addr, ClientRole::Broadcast, CODE);
        wait_for(&c, |e| matches!(e, NetEvent::State { snapshot: true, .. }));
        for a in [
            start_game(),
            Action::ManualAdjust {
                deltas: [5000, 0, 0, -5000],
                reason: "t".into(),
            },
            Action::EndGame,
        ] {
            let id = c.send_action(a);
            assert_eq!(wait_result(&c, id), None);
        }
        let e = wait_for(&c, |e| matches!(e, NetEvent::State { seq: 3, .. }));
        let NetEvent::State { view, .. } = e else {
            unreachable!()
        };
        let TableStateView::Full(f) = *view else {
            panic!()
        };
        assert_eq!(f.tournament.games.len(), 1, "終局した半荘が記録される");
        assert_eq!(f.tournament.standings()[0].name, "選手A");
        assert!(dir2.join("tournament.json").exists());
        // 終局を取り消すと記録も消える
        let id = c.undo();
        assert_eq!(wait_result(&c, id), None);
        let e = wait_for(&c, |e| matches!(e, NetEvent::State { seq: 4, .. }));
        let NetEvent::State { view, .. } = e else {
            unreachable!()
        };
        let TableStateView::Full(f) = *view else {
            panic!()
        };
        assert!(f.tournament.games.is_empty());
        // やり直すと戻る。名前の変更
        let id = c.redo();
        assert_eq!(wait_result(&c, id), None);
        let id = c.tournament(janscope_core::protocol::TournamentOp::Rename {
            name: "春季大会".into(),
        });
        assert_eq!(wait_result(&c, id), None);
    })
    .await
    .unwrap();
    srv.stop();
    // 再起動しても大会通算は残る
    let saved: janscope_core::tournament::Tournament =
        serde_json::from_str(&std::fs::read_to_string(dir.join("tournament.json")).unwrap())
            .unwrap();
    assert_eq!(saved.name, "春季大会");
    assert_eq!(saved.games.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

/// --dummy: ダミー対局データ（局の途中）で始まり、取り消しもできる
#[tokio::test(flavor = "multi_thread")]
async fn dummy_game_data() {
    let dir = tmp_dir("dummy");
    let mut c = cfg(&dir, 0, None);
    c.dummy = true;
    let running = start(c).await.unwrap();
    let addr = running.addr;
    tokio::task::spawn_blocking(move || {
        let mut cl = client(addr, ClientRole::Broadcast, CODE);
        let e = wait_for(&cl, |e| full_state(e).is_some());
        let s = full_state(&e).unwrap();
        assert_eq!(s.seats[3].player.name, "高橋 美咲");
        assert!(s.seats[3].riichi, "北家のリーチの途中");
        // 取り消すとリーチが外れる（ダミーも普通の操作の列）
        let id = cl.undo();
        assert_eq!(wait_result(&cl, id), None);
        wait_for(&cl, |e| full_state(e).is_some_and(|s| !s.seats[3].riichi));
    })
    .await
    .unwrap();
    running.stop();
}
