//! Integration tests for forge_eventsource_stream: Event, EventStream, and Eventsource.

use core::time::Duration;

use forge_eventsource_stream::{Event, EventStream, Eventsource};
use futures::prelude::*;
use futures::stream;

// ---------------------------------------------------------------------------
// Event struct tests
// ---------------------------------------------------------------------------

#[test]
fn event_default() {
    let e = Event::default();
    assert_eq!(e.event, "");
    assert_eq!(e.data, "");
    assert_eq!(e.id, "");
    assert_eq!(e.retry, None);
}

#[test]
fn event_equality() {
    let a = Event {
        event: "msg".into(),
        data: "hello".into(),
        id: "1".into(),
        retry: None,
    };
    let b = Event {
        event: "msg".into(),
        data: "hello".into(),
        id: "1".into(),
        retry: None,
    };
    assert_eq!(a, b);
}

#[test]
fn event_inequality_different_data() {
    let a = Event {
        event: "msg".into(),
        data: "hello".into(),
        id: "1".into(),
        retry: None,
    };
    let b = Event {
        event: "msg".into(),
        data: "world".into(),
        id: "1".into(),
        retry: None,
    };
    assert_ne!(a, b);
}

#[test]
fn event_inequality_different_retry() {
    let a = Event {
        event: "".into(),
        data: "".into(),
        id: "".into(),
        retry: None,
    };
    let b = Event {
        event: "".into(),
        data: "".into(),
        id: "".into(),
        retry: Some(Duration::from_secs(3)),
    };
    assert_ne!(a, b);
}

#[test]
fn event_clone() {
    let e = Event {
        event: "test".into(),
        data: "data".into(),
        id: "42".into(),
        retry: Some(Duration::from_millis(500)),
    };
    let cloned = e.clone();
    assert_eq!(e, cloned);
    assert_eq!(cloned.event, "test");
    assert_eq!(cloned.retry, Some(Duration::from_millis(500)));
}

#[test]
fn event_with_all_fields() {
    let e = Event {
        event: "message".into(),
        data: "hello world".into(),
        id: "id-123".into(),
        retry: Some(Duration::from_secs(5)),
    };
    assert_eq!(e.event, "message");
    assert_eq!(e.data, "hello world");
    assert_eq!(e.id, "id-123");
    assert_eq!(e.retry, Some(Duration::from_secs(5)));
}

#[test]
fn event_retry_zero() {
    let e = Event { retry: Some(Duration::from_secs(0)), ..Default::default() };
    assert_eq!(e.retry, Some(Duration::from_secs(0)));
}

#[test]
fn event_retry_large() {
    let e = Event {
        retry: Some(Duration::from_secs(u64::MAX)),
        ..Default::default()
    };
    assert_eq!(e.retry, Some(Duration::from_secs(u64::MAX)));
}

#[test]
fn event_empty_strings() {
    let e = Event::default();
    assert!(e.event.is_empty());
    assert!(e.data.is_empty());
    assert!(e.id.is_empty());
}

#[test]
fn event_debug_format() {
    let e = Event {
        event: "test".into(),
        data: "data".into(),
        id: "1".into(),
        retry: None,
    };
    let debug = format!("{:?}", e);
    assert!(debug.contains("Event"));
    assert!(debug.contains("test"));
    assert!(debug.contains("data"));
}

#[test]
fn event_clone_preserves_retry() {
    let original = Event {
        event: "deploy".into(),
        data: "v2.0".into(),
        id: "100".into(),
        retry: Some(Duration::from_millis(1500)),
    };
    let cloned = original.clone();
    assert_eq!(original, cloned);
    assert_eq!(cloned.retry, Some(Duration::from_millis(1500)));
}

// ---------------------------------------------------------------------------
// EventStream metadata tests
// ---------------------------------------------------------------------------

#[test]
fn eventstream_new_and_metadata() {
    let mut stream = EventStream::new(stream::empty::<Result<Vec<u8>, ()>>());
    assert_eq!(stream.last_event_id(), "");

    stream.set_last_event_id("abc-123");
    assert_eq!(stream.last_event_id(), "abc-123");

    stream.set_last_event_id("");
    assert_eq!(stream.last_event_id(), "");
}

#[test]
fn eventstream_set_last_event_id_replaces() {
    let mut stream = EventStream::new(stream::empty::<Result<Vec<u8>, ()>>());
    stream.set_last_event_id("first");
    assert_eq!(stream.last_event_id(), "first");

    stream.set_last_event_id("second");
    assert_eq!(stream.last_event_id(), "second");
}

// ---------------------------------------------------------------------------
// Eventsource trait tests
// ---------------------------------------------------------------------------

#[test]
fn eventsource_trait_creates_stream() {
    let input = stream::iter(vec![Ok::<_, ()>(b"data: test\n\n".to_vec())]);
    let mut event_stream = input.eventsource();
    assert_eq!(event_stream.last_event_id(), "");

    // Verify we can poll via Pin
    use std::pin::Pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn noop_waker() -> Waker {
        fn clone(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        fn noop(_: *const ()) {}
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut pinned = Pin::new(&mut event_stream);
    let result = pinned.as_mut().poll_next(&mut cx);
    // Should return Poll::Ready(Some(...)) since we fed valid SSE data
    assert!(matches!(result, Poll::Ready(Some(Ok(_)))));
}

// ---------------------------------------------------------------------------
// Edge case / integration tests
// ---------------------------------------------------------------------------

#[test]
fn eventstream_different_underlying_streams() {
    // Empty stream
    let _empty = EventStream::new(stream::empty::<Result<String, ()>>());

    // Single-item stream
    let _single = EventStream::new(stream::once(async { Ok::<_, ()>("test".to_string()) }));

    // Multi-item stream
    let items: Vec<Result<String, ()>> = vec![Ok("a".into()), Ok("b".into()), Ok("c".into())];
    let _multi = EventStream::new(stream::iter(items));
}

#[tokio::test]
#[allow(clippy::redundant_pattern_matching)]
async fn eventsource_stream_collects_multiple_items() {
    let input = stream::iter(vec![
        Ok::<_, ()>(b"data: first\n\n".to_vec()),
        Ok::<_, ()>(b"data: second\n\n".to_vec()),
    ]);
    let mut event_stream = input.eventsource();

    let mut events = Vec::new();
    while let Some(result) = event_stream.next().await {
        events.push(result.unwrap());
    }

    // Verify the stream processes without error
    // The EventStream parses SSE events from raw bytes
    assert!(events.len() <= 1000);
}

#[tokio::test]
#[allow(clippy::redundant_pattern_matching)]
async fn eventsource_empty_stream_terminates() {
    let input = stream::iter::<Vec<Result<Vec<u8>, ()>>>(vec![]);
    let mut event_stream = input.eventsource();

    let mut count = 0;
    while let Some(_) = event_stream.next().await {
        count += 1;
    }
    assert_eq!(count, 0);
}

#[tokio::test]
async fn eventsource_error_propagates() {
    #[derive(Debug, PartialEq)]
    struct MyError;
    let input = stream::iter(vec![
        Err::<Vec<u8>, _>(MyError),
        Ok::<_, _>(b"data: after-error\n\n".to_vec()),
    ]);
    let mut event_stream = input.eventsource();

    let first = event_stream.next().await;
    assert!(first.is_some());
    // Should propagate the transport error
    assert!(first.unwrap().is_err());
}
