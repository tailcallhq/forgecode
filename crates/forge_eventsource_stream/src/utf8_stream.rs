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
                    // A byte sequence that can never become valid: waiting
                    // for more input would only swallow every later chunk.
                    Err(err) if err.utf8_error().error_len().is_some() => {
                        *this.terminated = true;
                        Poll::Ready(Some(Err(Utf8StreamError::Utf8(err))))
                    }
                    // Only the tail is incomplete: a character split across
                    // chunks. Emit the valid prefix and keep the tail.
                    Err(err) => {
                        let valid_size = err.utf8_error().valid_up_to();
                        let mut bytes = err.into_bytes();
                        *this.buffer = bytes.split_off(valid_size);
                        // The prefix is valid by construction, so this never
                        // fails; checking it again keeps the code free of
                        // `unsafe`.
                        Poll::Ready(Some(
                            String::from_utf8(bytes).map_err(Utf8StreamError::Utf8),
                        ))
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
    use pretty_assertions::assert_eq;

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

    #[tokio::test]
    async fn invalid_byte_fails_at_once_instead_of_swallowing_later_chunks() {
        // 0xFF can never start a UTF-8 character, so waiting for more bytes
        // cannot fix it. The stream ends with the error; the valid prefix
        // stays inside the error's bytes.
        let fixture = futures::stream::iter(vec![
            Ok::<_, ()>(vec![b'a', 0xFF, b'b']),
            Ok::<_, ()>(b"later".to_vec()),
        ]);

        let actual = Utf8Stream::new(fixture).collect::<Vec<_>>().await;

        let expected = vec![Err(Utf8StreamError::Utf8(
            String::from_utf8(vec![b'a', 0xFF, b'b']).unwrap_err(),
        ))];
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn split_character_followed_by_invalid_byte_fails() {
        // The first chunk ends inside a 4-byte character, so it is held back;
        // the next byte cannot continue it.
        let fixture = futures::stream::iter(vec![
            Ok::<_, ()>(vec![240, 159]),
            Ok::<_, ()>(vec![b'A']),
            Ok::<_, ()>(b"later".to_vec()),
        ]);

        let actual = Utf8Stream::new(fixture).collect::<Vec<_>>().await;

        let expected = vec![
            Ok(String::new()),
            Err(Utf8StreamError::Utf8(
                String::from_utf8(vec![240, 159, b'A']).unwrap_err(),
            )),
        ];
        assert_eq!(actual, expected);
    }
}
