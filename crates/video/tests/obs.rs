//! OBS 連携のテスト。obs-websocket v5 の振る舞いをまねた小さなサーバーを立てて確かめる。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use janscope_video::obs::{ObsConfig, ObsLink, ObsStatus, ObsSwitchMode, auth_string};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

const SALT: &str = "c2FsdHNhbHQ=";
const CHALLENGE: &str = "Y2hhbGxlbmdl";

/// 偽の OBS。受け取ったリクエスト（requestType, requestData）を記録する。
async fn fake_obs(password: &'static str) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    tokio::spawn(async move {
        loop {
            let (s, _) = listener.accept().await.unwrap();
            let log = log2.clone();
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
                let hello = json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1,
                    "authentication": {"challenge": CHALLENGE, "salt": SALT}}});
                ws.send(Message::text(hello.to_string())).await.unwrap();
                while let Some(Ok(m)) = ws.next().await {
                    let Message::Text(t) = m else { continue };
                    let v: Value = serde_json::from_str(&t).unwrap();
                    match v["op"].as_u64() {
                        Some(1) => {
                            if v["d"]["authentication"].as_str()
                                != Some(auth_string(password, SALT, CHALLENGE).as_str())
                            {
                                let _ = ws.close(None).await;
                                return;
                            }
                            ws.send(Message::text(
                                json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string(),
                            ))
                            .await
                            .unwrap();
                        }
                        Some(6) => {
                            let d = &v["d"];
                            let ty = d["requestType"].as_str().unwrap().to_string();
                            log.lock()
                                .unwrap()
                                .push((ty.clone(), d["requestData"].clone()));
                            let resp = match ty.as_str() {
                                "GetSceneItemId" => {
                                    let name = d["requestData"]["sourceName"].as_str().unwrap();
                                    json!({"sceneItemId": 100 + name.chars().last().unwrap() as i64 - 'A' as i64})
                                }
                                _ => json!({}),
                            };
                            let r = json!({"op": 7, "d": {"requestType": ty, "requestId": d["requestId"],
                                "requestStatus": {"result": true, "code": 100}, "responseData": resp}});
                            ws.send(Message::text(r.to_string())).await.unwrap();
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    (url, log)
}

fn wait(mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn scene_mode_with_password() {
    let (url, log) = fake_obs("pw").await;
    tokio::task::spawn_blocking(move || {
        let link = ObsLink::start(ObsConfig {
            enabled: true,
            url,
            password: "pw".into(),
            ..Default::default()
        });
        assert!(
            wait(|| matches!(link.status(), ObsStatus::Connected { .. })),
            "{:?}",
            link.status()
        );
        link.show_seat(2);
        assert!(wait(|| link.applied_seat() == Some(2)));
        link.show_seat(2); // 同じ席は送らない
        link.show_seat(0);
        assert!(wait(|| link.applied_seat() == Some(0)));
        let l = log.lock().unwrap().clone();
        let scenes: Vec<&str> = l
            .iter()
            .map(|(_, d)| d["sceneName"].as_str().unwrap())
            .collect();
        assert_eq!(scenes, vec!["顔C", "顔A"]);
        assert!(l.iter().all(|(t, _)| t == "SetCurrentProgramScene"));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_reported() {
    let (url, _log) = fake_obs("right").await;
    tokio::task::spawn_blocking(move || {
        let link = ObsLink::start(ObsConfig {
            enabled: true,
            url,
            password: "wrong".into(),
            ..Default::default()
        });
        assert!(wait(|| matches!(link.status(), ObsStatus::Error(_))));
        assert!(link.last_error().unwrap().contains("認証"));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn source_visibility_mode() {
    let (url, log) = fake_obs("").await;
    tokio::task::spawn_blocking(move || {
        let link = ObsLink::start(ObsConfig {
            enabled: true,
            url,
            mode: ObsSwitchMode::SourceVisibility,
            ..Default::default()
        });
        link.show_seat(1);
        assert!(
            wait(|| link.applied_seat() == Some(1)),
            "{:?}",
            link.last_error()
        );
        let l = log.lock().unwrap().clone();
        let gets = l.iter().filter(|(t, _)| t == "GetSceneItemId").count();
        assert_eq!(gets, 4);
        let sets: Vec<(i64, bool)> = l
            .iter()
            .filter(|(t, _)| t == "SetSceneItemEnabled")
            .map(|(_, d)| {
                (
                    d["sceneItemId"].as_i64().unwrap(),
                    d["sceneItemEnabled"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            sets,
            vec![(100, false), (101, true), (102, false), (103, false)]
        );
    })
    .await
    .unwrap();
}
