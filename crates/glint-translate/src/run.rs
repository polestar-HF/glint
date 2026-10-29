//! Running several engines at once.

use std::borrow::Borrow;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::task::Poll;

use tokio::time::Instant;

use crate::{TranslateRequest, Translator};

/// What happened to one engine. The app maps these one-to-one onto the `tr:start`,
/// `tr:delta`, `tr:done` and `tr:error` events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    Start {
        engine_id: String,
        engine_name: String,
    },
    Delta {
        engine_id: String,
        delta: String,
    },
    Done {
        engine_id: String,
        text: String,
        elapsed_ms: u64,
    },
    /// `message` is short and fit to show to the user.
    Error {
        engine_id: String,
        message: String,
    },
}

impl EngineEvent {
    pub fn engine_id(&self) -> &str {
        match self {
            EngineEvent::Start { engine_id, .. }
            | EngineEvent::Delta { engine_id, .. }
            | EngineEvent::Done { engine_id, .. }
            | EngineEvent::Error { engine_id, .. } => engine_id,
        }
    }
}

type BoxFut<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Run every engine concurrently on the same request and report progress through `on_event`.
///
/// Each engine produces `Start`, then any number of `Delta`s, then exactly one of `Done` or
/// `Error`. Events from different engines interleave; events of one engine arrive in order.
/// `on_event` is called from the task that polls this future, never from another thread.
///
/// The returned future resolves when every engine has finished. It owns all the work, so
/// dropping it (or aborting the task that awaits it) cancels every in-flight request.
///
/// `engines` can hold `Box<dyn Translator>` or `Arc<dyn Translator>`.
pub async fn translate_all<E, F>(engines: &[E], req: &TranslateRequest, on_event: F)
where
    E: Borrow<dyn Translator> + Sync,
    F: Fn(EngineEvent) + Send + Sync,
{
    let on_event = &on_event;
    let mut running: Vec<Option<BoxFut<'_>>> = engines
        .iter()
        .map(|e| {
            let engine: &dyn Translator = e.borrow();
            Some(Box::pin(run_one(engine, req, on_event)) as BoxFut<'_>)
        })
        .collect();

    // A small join-all: poll every unfinished engine whenever any of them is woken. There are
    // only a handful of engines, so this is cheaper than spawning tasks, and it keeps the
    // borrowed request and callback alive without `'static` bounds.
    poll_fn(move |cx| {
        let mut pending = false;
        for slot in running.iter_mut() {
            if let Some(fut) = slot {
                if fut.as_mut().poll(cx).is_ready() {
                    *slot = None;
                } else {
                    pending = true;
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
}

async fn run_one<F>(engine: &dyn Translator, req: &TranslateRequest, on_event: &F)
where
    F: Fn(EngineEvent) + Send + Sync,
{
    let id = engine.id().to_string();
    on_event(EngineEvent::Start {
        engine_id: id.clone(),
        engine_name: engine.name().to_string(),
    });
    let started = Instant::now();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let work = engine.translate(req, tx);
    tokio::pin!(work);

    let result = loop {
        tokio::select! {
            // Deltas first, so text is never reported after the engine's own `Done`.
            biased;
            Some(delta) = rx.recv() => {
                on_event(EngineEvent::Delta { engine_id: id.clone(), delta });
            }
            res = &mut work => break res,
        }
    };
    // The engine may have queued a last delta right before returning.
    while let Ok(delta) = rx.try_recv() {
        on_event(EngineEvent::Delta {
            engine_id: id.clone(),
            delta,
        });
    }

    match result {
        Ok(text) => on_event(EngineEvent::Done {
            engine_id: id,
            text,
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        }),
        Err(e) => {
            log::warn!("engine {id} failed: {e}");
            on_event(EngineEvent::Error {
                engine_id: id,
                message: e.user_message(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeltaSink, Result, TranslateError};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Sends the given deltas with a pause between them, then returns `result`.
    struct Fake {
        id: &'static str,
        deltas: Vec<&'static str>,
        pause_ms: u64,
        fail: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl Translator for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            "Fake"
        }
        async fn translate(&self, _req: &TranslateRequest, deltas: DeltaSink) -> Result<String> {
            let mut all = String::new();
            for d in &self.deltas {
                tokio::time::sleep(Duration::from_millis(self.pause_ms)).await;
                let _ = deltas.send((*d).to_string());
                all.push_str(d);
            }
            match self.fail {
                Some(m) => Err(TranslateError::Service {
                    status: 401,
                    message: m.into(),
                }),
                None => Ok(all),
            }
        }
    }

    fn collect() -> (
        Arc<Mutex<Vec<EngineEvent>>>,
        impl Fn(EngineEvent) + Send + Sync,
    ) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        (log, move |e| {
            sink.lock().unwrap_or_else(|p| p.into_inner()).push(e)
        })
    }

    fn events_of(log: &Mutex<Vec<EngineEvent>>, id: &str) -> Vec<EngineEvent> {
        log.lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|e| e.engine_id() == id)
            .cloned()
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn emits_start_deltas_and_done_per_engine_in_order() {
        let engines: Vec<Box<dyn Translator>> = vec![
            Box::new(Fake {
                id: "a",
                deltas: vec!["he", "llo"],
                pause_ms: 10,
                fail: None,
            }),
            Box::new(Fake {
                id: "b",
                deltas: vec!["x"],
                pause_ms: 5,
                fail: Some("Invalid API key"),
            }),
            Box::new(Fake {
                id: "c",
                deltas: vec![],
                pause_ms: 0,
                fail: None,
            }),
        ];
        let (log, on_event) = collect();
        translate_all(&engines, &TranslateRequest::default(), on_event).await;

        let a = events_of(&log, "a");
        assert_eq!(a.len(), 4, "{a:?}");
        assert_eq!(
            a[0],
            EngineEvent::Start {
                engine_id: "a".into(),
                engine_name: "Fake".into()
            }
        );
        assert_eq!(
            a[1],
            EngineEvent::Delta {
                engine_id: "a".into(),
                delta: "he".into()
            }
        );
        assert_eq!(
            a[2],
            EngineEvent::Delta {
                engine_id: "a".into(),
                delta: "llo".into()
            }
        );
        match &a[3] {
            EngineEvent::Done {
                text, elapsed_ms, ..
            } => {
                assert_eq!(text, "hello");
                assert_eq!(*elapsed_ms, 20);
            }
            other => panic!("expected Done, got {other:?}"),
        }

        let b = events_of(&log, "b");
        assert_eq!(b.len(), 3, "{b:?}");
        assert_eq!(
            b[2],
            EngineEvent::Error {
                engine_id: "b".into(),
                message: "Invalid API key".into()
            }
        );

        let c = events_of(&log, "c");
        assert!(matches!(c.last(), Some(EngineEvent::Done { text, .. }) if text.is_empty()));
    }

    #[tokio::test(start_paused = true)]
    async fn engines_run_concurrently() {
        let engines: Vec<Box<dyn Translator>> = (0..5)
            .map(|i| {
                Box::new(Fake {
                    id: ["e0", "e1", "e2", "e3", "e4"][i],
                    deltas: vec!["a", "b"],
                    pause_ms: 100,
                    fail: None,
                }) as Box<dyn Translator>
            })
            .collect();
        let (_log, on_event) = collect();
        let t0 = tokio::time::Instant::now();
        translate_all(&engines, &TranslateRequest::default(), on_event).await;
        // Five engines of 200 ms each take 200 ms, not 1000 ms.
        assert_eq!(t0.elapsed(), Duration::from_millis(200));
    }

    #[tokio::test(start_paused = true)]
    async fn accepts_arc_engines_and_an_empty_list() {
        let engines: Vec<Arc<dyn Translator>> = vec![Arc::new(Fake {
            id: "a",
            deltas: vec!["x"],
            pause_ms: 1,
            fail: None,
        })];
        let (log, on_event) = collect();
        translate_all(&engines, &TranslateRequest::default(), on_event).await;
        assert_eq!(events_of(&log, "a").len(), 3);

        let none: Vec<Box<dyn Translator>> = Vec::new();
        let (log, on_event) = collect();
        translate_all(&none, &TranslateRequest::default(), on_event).await;
        assert!(log.lock().map(|l| l.is_empty()).unwrap_or(false));
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_future_cancels_every_engine() {
        let engines: Vec<Box<dyn Translator>> = vec![
            Box::new(Fake {
                id: "slow",
                deltas: vec!["a", "b", "c"],
                pause_ms: 1000,
                fail: None,
            }),
            Box::new(Fake {
                id: "fast",
                deltas: vec!["a"],
                pause_ms: 10,
                fail: None,
            }),
        ];
        let (log, on_event) = collect();
        let req = TranslateRequest::default();
        let res = tokio::time::timeout(
            Duration::from_millis(1500),
            translate_all(&engines, &req, on_event),
        )
        .await;
        assert!(res.is_err(), "the timeout must fire first");
        let slow = events_of(&log, "slow");
        // Started, got the first delta at 1000 ms, then was cancelled at 1500 ms.
        assert_eq!(slow.len(), 2, "{slow:?}");
        assert!(matches!(
            events_of(&log, "fast").last(),
            Some(EngineEvent::Done { .. })
        ));

        // Nothing is reported after the drop.
        let before = log.lock().map(|l| l.len()).unwrap_or(0);
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(log.lock().map(|l| l.len()).unwrap_or(0), before);
    }
}
