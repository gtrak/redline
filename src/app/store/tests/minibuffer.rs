use super::*;

    #[test]
    fn unknown_keys_echo_in_minibuffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store(dir.path());
        store.key_event(key("Backspace"));
        assert_eq!(store.message, "unbound key: DEL");
        assert!(store.pending.is_empty());
    }

