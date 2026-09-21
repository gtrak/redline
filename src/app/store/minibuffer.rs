use super::*;

impl AppStore {
    pub fn minibuffer_message(&mut self, msg: &str) {
        self.message = msg.to_string();
    }
}
