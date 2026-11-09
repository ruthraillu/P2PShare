use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{broadcast, watch},
};
use tokio_websockets::{Limits, Message, ServerBuilder};

pub struct Relay {
    pub url: String,
    stop: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub accepted: Arc<std::sync::atomic::AtomicUsize>,
    pub records: Arc<Mutex<HashMap<String, Value>>>,
}
impl Relay {
    pub fn start() -> Self {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop, mut shutdown) = watch::channel(false);
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = accepted.clone();
        let records = Arc::new(Mutex::new(HashMap::<String, Value>::new()));
        let stored = records.clone();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                ready_tx.send(format!("ws://{}",listener.local_addr().unwrap())).unwrap();
                let records = stored;
                let (updates,_) = broadcast::channel::<Value>(128);
                loop {
                    tokio::select! {
                        _ = shutdown.changed() => break,
                        connection = listener.accept() => {
                            let (stream,_) = connection.unwrap();
                            let records = records.clone(); let updates = updates.clone(); let counter = counter.clone();
                            tokio::spawn(async move {
                                let Ok((_,mut ws)) = ServerBuilder::new().limits(Limits::default().max_payload_len(Some(64*1024))).accept(stream).await else { return; };
                                let mut rx = updates.subscribe();
                                let mut subscription: Option<(String,String)> = None;
                                loop {
                                    tokio::select! {
                                        message = ws.next() => {
                                            let Some(Ok(message)) = message else { break; };
                                            if message.is_close() { break; }
                                            let Some(text) = message.as_text() else { continue; };
                                            let Ok(v) = serde_json::from_str::<Value>(text) else { continue; };
                                            match v[0].as_str() {
                                                Some("REQ") => {
                                                    let sub = v[1].as_str().unwrap().to_string(); let topic = v[2]["#t"][0].as_str().unwrap().to_string();
                                                    subscription = Some((sub.clone(),topic.clone()));
                                                    let stored: Vec<_> = records.lock().unwrap().values().filter(|e| matches_topic(e,&topic)).cloned().collect();
                                                    for e in stored { if ws.send(Message::text(json!(["EVENT",sub,e]).to_string())).await.is_err() { return; } }
                                                    if ws.send(Message::text(json!(["EOSE",sub]).to_string())).await.is_err() { return; }
                                                }
                                                Some("EVENT") => {
                                                    let e = &v[1];
                                                    let d = e["tags"].as_array().unwrap().iter().find(|t| t[0]=="d").unwrap()[1].as_str().unwrap();
                                                    let key = format!("{}:{}:{}",e["kind"],e["pubkey"],d);
                                                    {
                                                        let mut stored = records.lock().unwrap();
                                                        if stored.get(&key).is_none_or(|old| (old["created_at"].as_u64(),std::cmp::Reverse(old["id"].as_str())) < (e["created_at"].as_u64(),std::cmp::Reverse(e["id"].as_str()))) { stored.insert(key,e.clone()); }
                                                    }
                                                    counter.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                                                    if ws.send(Message::text(json!(["OK",e["id"],true,""]).to_string())).await.is_err() { return; }
                                                    let _ = updates.send(e.clone());
                                                }
                                                _ => {}
                                            }
                                        }
                                        e = rx.recv() => {
                                            if let Ok(e) = e && let Some((sub,topic)) = &subscription && matches_topic(&e,topic) && ws.send(Message::text(json!(["EVENT",sub,e]).to_string())).await.is_err() { break; }
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            });
        });
        let url = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            url,
            stop,
            thread: Some(thread),
            accepted,
            records,
        }
    }
    pub fn shutdown(&mut self) {
        let _ = self.stop.send(true);
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn matches_topic(event: &Value, topic: &str) -> bool {
    event["kind"] == 30378
        && event["tags"]
            .as_array()
            .is_some_and(|tags| tags.iter().any(|t| t[0] == "t" && t[1] == topic))
}
