use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc, Mutex},
};

pub struct PendingResponses {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    pending: HashMap<Value, mpsc::Sender<Value>>,
    owned: HashSet<Value>,
}

impl PendingResponses {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }

    pub fn register(&self, id: Value) -> mpsc::Receiver<Value> {
        let (sender, receiver) = mpsc::channel();
        let mut state = self.state.lock().unwrap();
        state.owned.insert(id.clone());
        state.pending.insert(id, sender);
        receiver
    }

    pub fn remove(&self, id: &Value) {
        let mut state = self.state.lock().unwrap();
        state.pending.remove(id);
    }

    /// Returns true when the response belongs to the proxy and must not be
    /// forwarded to the editor, including responses that arrive after timeout.
    pub fn route(&self, message: &Value) -> bool {
        if message.get("method").is_some() {
            return false;
        }
        let Some(id) = message.get("id") else {
            return false;
        };

        let (sender, owned) = {
            let mut state = self.state.lock().unwrap();
            let sender = state.pending.remove(id);
            let owned = state.owned.remove(id);
            (sender, owned)
        };
        if let Some(sender) = sender {
            let _ = sender.send(message.clone());
            return true;
        }

        owned
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.pending.clear();
        state.owned.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn routes_each_pending_response_once() {
        let pending = PendingResponses::new();
        let id = json!("proxy-owned-1");
        let receiver = pending.register(id.clone());
        let response = json!({ "jsonrpc": "2.0", "id": id, "result": "ok" });

        assert!(pending.route(&response));
        assert_eq!(receiver.recv().unwrap(), response);
        assert!(!pending.route(&response));
    }

    #[test]
    fn swallows_late_owned_responses_but_not_client_responses() {
        let pending = PendingResponses::new();
        let retired = json!("proxy-owned-late");
        pending.register(retired.clone());
        pending.remove(&retired);

        let response = json!({
            "jsonrpc": "2.0",
            "id": "proxy-owned-late",
            "result": null
        });
        assert!(pending.route(&response));
        assert!(!pending.route(&response));
        assert!(!pending.route(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": null
        })));
        assert!(!pending.route(&json!({
            "jsonrpc": "2.0",
            "id": "proxy-owned-server-request",
            "method": "workspace/applyEdit",
            "params": {}
        })));
    }

    #[test]
    fn does_not_claim_unregistered_prefixed_ids() {
        let pending = PendingResponses::new();

        assert!(!pending.route(&json!({
            "jsonrpc": "2.0",
            "id": "proxy-owned-editor-request",
            "result": null
        })));
    }

    #[test]
    fn ownership_is_retained_until_a_response_arrives() {
        let pending = PendingResponses::new();
        for id in 0..=1024 {
            let id = json!(format!("proxy-owned-{id}"));
            pending.register(id.clone());
            pending.remove(&id);
        }

        assert!(pending.route(&json!({
            "jsonrpc": "2.0",
            "id": "proxy-owned-0",
            "result": null
        })));
    }
}
