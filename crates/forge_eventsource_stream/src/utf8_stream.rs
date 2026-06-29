#[cfg(not(feature = "std"))]
use alloc::{
    string::{FromUtf8Error, String},
    vec::Vec,
};
use core::pin::Pin;
#[cfg(feature = "std")]
use std::string::FromUtf8Error;

use futures_core::stream::Stream;
use futures_core::task::{Context, Poll};
use pin_project_lite::pin_project;

/// Maximum number of bytes buffered for a single partial UTF-8 sequence.
/// Sequences can be at most 4 bytes; this cap prevents unbounded growth on
/// malformed or adversarial input streams.
const MAX_UTF8_BUFFER: usize = 4 * 1024; // 4 KiB — far more than any valid sequence

pin_project! {
pub struct Utf8Stream<S> {
    #[pin]
    stream: S,
    buffer: Vec<u8>,
    terminated: bool,
}
}

impl<S> Utf8Stream<S> {
    pub fn new(stream: S) -> Self {
        Self { stream, buffer: Vec::new(), terminated: false }
    }
}

#[derive(Debug, PartialEq)]
pub enum Utf8StreamError<E> {
    Utf8(FromUtf8Error),
    Transport(E),
}

impl<E> From<FromUtf8Error> for Utf8StreamError<E> {
    fn from(err: FromUtf8Error) -> Self {
        Self::Utf8(err)
    }
}

impl<S, B, E> Stream for Utf8Stream<S>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    type Item = Result<String, Utf8StreamError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.project();
        if *this.terminated {
            return Poll::Ready(None);
        }
        match this.stream.poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                this.buffer.extend_from_slice(bytes.as_ref());
                let bytes = core::mem::take(this.buffer);
                match String::from_utf8(bytes) {
                    Ok(string) => Poll::Ready(Some(Ok(string))),
                    Err(err) => {
                        let valid_size = err.utf8_error().valid_up_to();
                        let mut bytes = err.into_bytes();
                        let rem = bytes.split_off(valid_size);
                        // A valid UTF-8 partial-sequence remainder is at most 3
                        // bytes. If the remainder exceeds MAX_UTF8_BUFFER, the
                        // stream is malformed; emit an error and clear the
                        // buffer to prevent unbounded accumulation.
                        if rem.len() > MAX_UTF8_BUFFER {
                            return Poll::Ready(Some(Err(Utf8StreamError::Utf8(
                                String::from_utf8(rem).unwrap_err(),
                            ))));
                        }
                        *this.buffer = rem;
                        // SAFETY: `bytes` contains exactly the validated UTF-8
                        // prefix of the original slice; `valid_up_to()` guarantees
                        // all bytes in `[0, valid_size)` are valid UTF-8.
                        Poll::Ready(Some(Ok(unsafe { String::from_utf8_unchecked(bytes) })))
                    }
                }
            }
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(Utf8StreamError::Transport(err)))),
            Poll::Ready(None) => {
                *this.terminated = true;
                if this.buffer.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(
                        String::from_utf8(core::mem::take(this.buffer))
                            .map_err(Utf8StreamError::Utf8),
                    ))
                }
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::prelude::*;

    use super::*;

    #[tokio::test]
    async fn valid_streams() {
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(b"Hello, world!")]))
                .try_collect::<Vec<_>>()
                .await
                .unwrap(),
            vec!["Hello, world!"]
        );
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>("Hello, world!")]))
                .try_collect::<Vec<_>>()
                .await
                .unwrap(),
            vec!["Hello, world!"]
        );
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>("")]))
                .try_collect::<Vec<_>>()
                .await
                .unwrap(),
            vec![""]
        );
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>("Hello"),
                Ok::<_, ()>(", world!")
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec!["Hello", ", world!"]
        );
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(vec![
                240, 159, 145, 141
            ]),]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec!["👍"]
        );
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>(vec![240, 159]),
                Ok::<_, ()>(vec![145, 141])
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec!["", "👍"]
        );
        assert_eq!(
            Utf8Stream::new(futures::stream::iter(vec![
                Ok::<_, ()>(vec![240, 159]),
                Ok::<_, ()>(vec![145, 141, 240, 159, 145, 141])
            ]))
            .try_collect::<Vec<_>>()
            .await
            .unwrap(),
            vec!["", "👍👍"]
        );
    }

    #[tokio::test]
    async fn invalid_streams() {
        let results = Utf8Stream::new(futures::stream::iter(vec![Ok::<_, ()>(vec![240, 159])]))
            .collect::<Vec<_>>()
            .await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], Ok("".to_string()));
        assert!(matches!(results[1], Err(Utf8StreamError::Utf8(_))));

        let results = Utf8Stream::new(futures::stream::iter(vec![
            Ok::<_, ()>(vec![240, 159]),
            Ok::<_, ()>(vec![145, 141, 240, 159, 145]),
        ]))
        .collect::<Vec<_>>()
        .await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], Ok("".to_string()));
        assert_eq!(results[1], Ok("👍".to_string()));
        assert!(matches!(results[2], Err(Utf8StreamError::Utf8(_))));
    }
}
