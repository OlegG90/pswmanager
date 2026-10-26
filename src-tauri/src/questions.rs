//! A question the extension parks while the user decides (see `CONTEXT.md`):
//! the window asks, the pipe-serving thread waits, and the answer comes back
//! through a command. The registry owns the wait; the app's side only shows
//! the window and learns of the expiry, so tests drive it without Tauri.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// How long a question waits for the user; the extension gives up on its own
/// after a while. Owned here: the window only learns that it expired.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(120);

/// What the app does with a question: shows it in the window, and is told
/// when it expired unanswered, so the dialog goes.
pub trait Transport: Send + Sync {
    fn send(&self, event: &str, details: Value);
    fn expired(&self, id: u64);
}

/// The time a question waits on: the system's in the app, a scripted one in
/// tests.
pub trait Clock: Send + Sync {
    fn wait(&self, answered: &mpsc::Receiver<Value>, within: Duration) -> Result<Value, mpsc::RecvTimeoutError>;
}

/// The clock the app waits on.
pub struct SystemClock;

impl Clock for SystemClock {
    fn wait(&self, answered: &mpsc::Receiver<Value>, within: Duration) -> Result<Value, mpsc::RecvTimeoutError> {
        answered.recv_timeout(within)
    }
}

/// Questions waiting for the user's answer in the window, by number.
#[derive(Default)]
pub struct Questions {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::Sender<Value>>>,
}

impl Questions {
    /// Shows the window, asks it `event` with `details`, and waits for the
    /// answer; `Value::Null` when there is none in time.
    pub fn ask(&self, transport: &impl Transport, clock: &impl Clock, event: &str, mut details: Value) -> Value {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, answer);
        details["id"] = serde_json::json!(id);
        transport.send(event, details);
        let reply = clock.wait(&answered, ANSWER_WITHIN).unwrap_or(Value::Null);
        // Unanswered in time: the dialog goes, so the user does not answer in vain.
        if self.waiting.lock().unwrap().remove(&id).is_some() {
            transport.expired(id);
        }
        reply
    }

    /// The window's answer to question `id`; true when a question waited for
    /// it. A second or a late answer reaches nothing.
    pub fn answer(&self, id: u64, answer: Value) -> bool {
        if let Some(waiting) = self.waiting.lock().unwrap().remove(&id) {
            let _ = waiting.send(answer);
            true
        } else {
            false
        }
    }

    /// Drops the questions still waiting (the database locked): each is
    /// answered as cancelled, so an answer given after cannot reach the
    /// database opened next.
    pub fn forget(&self) {
        self.waiting.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// What the window was shown, and what expired: the test answers through
    /// [Questions::answer], as the command does.
    struct Scripted {
        sent: Mutex<Vec<(String, Value)>>,
        expired: Mutex<Vec<u64>>,
    }

    impl Scripted {
        fn new() -> Scripted {
            Scripted { sent: Mutex::new(Vec::new()), expired: Mutex::new(Vec::new()) }
        }

        fn asked(&self) -> u64 {
            let sent = self.sent.lock().unwrap();
            sent[0].1["id"].as_u64().unwrap()
        }
    }

    impl Transport for Scripted {
        fn send(&self, event: &str, details: Value) {
            self.sent.lock().unwrap().push((event.into(), details));
        }

        fn expired(&self, id: u64) {
            self.expired.lock().unwrap().push(id);
        }
    }

    /// A clock the test times out on demand: set, the wait ends at once.
    struct TestClock {
        timed_out: AtomicBool,
    }

    impl TestClock {
        fn waiting() -> TestClock {
            TestClock { timed_out: AtomicBool::new(false) }
        }

        fn timed_out() -> TestClock {
            TestClock { timed_out: AtomicBool::new(true) }
        }
    }

    impl Clock for TestClock {
        fn wait(&self, answered: &mpsc::Receiver<Value>, within: Duration) -> Result<Value, mpsc::RecvTimeoutError> {
            if self.timed_out.load(Ordering::SeqCst) {
                return Err(mpsc::RecvTimeoutError::Timeout);
            }
            // A dropped question disconnects the wait, as in the app.
            answered.recv_timeout(within).map_err(|_| mpsc::RecvTimeoutError::Timeout)
        }
    }

    /// Asks in a thread (as the pipe's thread does) and answers from here.
    fn asked(questions: &Arc<Questions>, transport: &Arc<Scripted>, clock: &Arc<TestClock>) -> std::thread::JoinHandle<Value> {
        let (questions, transport, clock) = (questions.clone(), transport.clone(), clock.clone());
        std::thread::spawn(move || questions.ask(&*transport, &*clock, "browser-associate", json!({})))
    }

    /// The question's number, once the window was shown it.
    fn shown(transport: &Scripted) -> u64 {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while transport.sent.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < end, "the window was never shown the question");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        transport.asked()
    }

    #[test]
    fn an_answer_reaches_the_question() {
        let (questions, transport, clock) = (Arc::new(Questions::default()), Arc::new(Scripted::new()), Arc::new(TestClock::waiting()));
        let asked = asked(&questions, &transport, &clock);
        let id = shown(&transport);
        assert!(questions.answer(id, json!("Named")), "the first answer reaches the question");
        assert_eq!(asked.join().unwrap(), json!("Named"));
    }

    #[test]
    fn a_second_answer_reaches_nothing() {
        let (questions, transport, clock) = (Arc::new(Questions::default()), Arc::new(Scripted::new()), Arc::new(TestClock::waiting()));
        let asked = asked(&questions, &transport, &clock);
        let id = shown(&transport);
        assert!(questions.answer(id, json!("Named")));
        assert!(!questions.answer(id, json!("Renamed")), "only the first answer reaches the question");
        assert_eq!(asked.join().unwrap(), json!("Named"));
    }

    #[test]
    fn an_answer_to_no_question_reaches_nothing() {
        let questions = Questions::default();
        assert!(!questions.answer(7, json!("Named")), "nothing waited for it");
    }

    #[test]
    fn unanswered_in_time_expires() {
        let (questions, transport, clock) = (Questions::default(), Scripted::new(), TestClock::timed_out());
        let reply = questions.ask(&transport, &clock, "browser-associate", json!({}));
        assert_eq!(reply, Value::Null, "no answer in time");
        assert_eq!(*transport.expired.lock().unwrap(), [transport.asked()], "the dialog goes");
        assert!(!questions.answer(transport.asked(), json!("Late")), "a late answer reaches nothing");
    }

    #[test]
    fn an_answer_after_forget_reaches_nothing() {
        let (questions, transport, clock) = (Arc::new(Questions::default()), Arc::new(Scripted::new()), Arc::new(TestClock::waiting()));
        let asked = asked(&questions, &transport, &clock);
        let id = shown(&transport);
        questions.forget();
        assert!(!questions.answer(id, json!("Late")), "the locked database is asked nothing");
        // Forgetting drops the wait, so the thread wakes as cancelled.
        assert_eq!(asked.join().unwrap(), Value::Null);
        assert!(transport.expired.lock().unwrap().is_empty(), "nothing was waiting to expire");
    }
}
