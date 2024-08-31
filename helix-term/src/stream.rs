use core::cmp::Ordering;
use core::fmt::{self, Debug};
use core::iter::FromIterator;
use core::pin::Pin;
use futures_util::stream::{FusedStream, FuturesUnordered, Stream};
use futures_util::task::{Context, Poll};
use futures_util::{ready, Future, StreamExt};
use std::collections::binary_heap::PeekMut;
use std::collections::{BinaryHeap, HashMap};

use pin_project_lite::pin_project;

pin_project! {
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    #[derive(Debug)]
    struct SeriesWrapper<T> {
        #[pin]
        data: T, // A future or a future's output
        // Use i64 for index since isize may overflow in 32-bit targets.
        index: i64,
    }
}

impl<T> PartialEq for SeriesWrapper<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T> Eq for SeriesWrapper<T> {}

impl<T> PartialOrd for SeriesWrapper<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for SeriesWrapper<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max heap, so compare backwards here.
        other.index.cmp(&self.index)
    }
}

impl<T> Future for SeriesWrapper<T>
where
    T: Future,
{
    type Output = SeriesWrapper<T::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let index = self.index;
        self.project().data.poll(cx).map(|output| SeriesWrapper {
            data: output,
            index,
        })
    }
}

/// An unbounded queue of futures.
///
/// This "combinator" is similar to [`FuturesUnordered`], but it imposes a FIFO
/// order on top of the set of futures. While futures in the set will race to
/// completion in parallel, results will only be returned in the order their
/// originating futures were added to the queue.
///
/// Futures are pushed into this queue and their realized values are yielded in
/// order. This structure is optimized to manage a large number of futures.
/// Futures managed by [`FuturesUnorderedSeries`] will only be polled when they generate
/// notifications. This reduces the required amount of work needed to coordinate
/// large numbers of futures.
///
/// When a [`FuturesUnorderedSeries`] is first created, it does not contain any futures.
/// Calling [`poll_next`](FuturesUnorderedSeries::poll_next) in this state will result
/// in [`Poll::Ready(None)`](Poll::Ready) to be returned. Futures are submitted
/// to the queue using [`push_back`](FuturesUnorderedSeries::push_back) (or
/// [`push_front`](FuturesUnorderedSeries::push_front)); however, the future will
/// **not** be polled at this point. [`FuturesUnorderedSeries`] will only poll managed
/// futures when [`FuturesUnorderedSeries::poll_next`] is called. As such, it
/// is important to call [`poll_next`](FuturesUnorderedSeries::poll_next) after pushing
/// new futures.
///
/// If [`FuturesUnorderedSeries::poll_next`] returns [`Poll::Ready(None)`](Poll::Ready)
/// this means that the queue is currently not managing any futures. A future
/// may be submitted to the queue at a later time. At that point, a call to
/// [`FuturesUnorderedSeries::poll_next`] will either return the future's resolved value
/// **or** [`Poll::Pending`] if the future has not yet completed. When
/// multiple futures are submitted to the queue, [`FuturesUnorderedSeries::poll_next`]
/// will return [`Poll::Pending`] until the first future completes, even if
/// some of the later futures have already completed.
///
/// Note that you can create a ready-made [`FuturesUnorderedSeries`] via the
/// [`collect`](Iterator::collect) method, or you can start with an empty queue
/// with the [`FuturesUnorderedSeries::new`] constructor.
///
/// This type is only available when the `std` or `alloc` feature of this
/// library is activated, and it is activated by default.
#[must_use = "streams do nothing unless polled"]
pub struct FuturesUnorderedSeries<T: Future> {
    in_progress_queue: FuturesUnordered<SeriesWrapper<T>>,
    series_queue: HashMap<i64, Vec<T>>,
    next_index: i64,
}

impl<T: Future> Unpin for FuturesUnorderedSeries<T> {}

impl<Fut: Future> FuturesUnorderedSeries<Fut> {
    /// Constructs a new, empty `FuturesUnorderedSeries`
    ///
    /// The returned [`FuturesUnorderedSeries`] does not contain any futures and, in
    /// this state, [`FuturesUnorderedSeries::poll_next`] will return
    /// [`Poll::Ready(None)`](Poll::Ready).
    pub fn new() -> Self {
        Self {
            in_progress_queue: FuturesUnordered::new(),
            series_queue: HashMap::new(),
            next_index: 0,
        }
    }

    /// Returns the number of futures contained in the queue.
    ///
    /// This represents the total number of in-flight futures, both
    /// those currently processing and those that are left in a series.
    pub fn len(&self) -> usize {
        self.in_progress_queue.len() + self.series_queue.values().fold(0, |acc, s| acc + s.len())
    }

    /// Returns `true` if the queue contains no futures
    pub fn is_empty(&self) -> bool {
        self.in_progress_queue.is_empty() && self.series_queue.is_empty()
    }

    /// Push a future into the queue.
    ///
    /// This function submits the given future to the internal set for managing.
    /// This function will not call [`poll`](Future::poll) on the submitted
    /// future. The caller must ensure that [`FuturesUnorderedSeries::poll_next`] is
    /// called in order to receive task notifications.
    pub fn push(&mut self, future: Fut) {
        let wrapped = SeriesWrapper {
            data: future,
            index: self.next_index,
        };
        self.next_index += 1;
        self.in_progress_queue.push(wrapped);
    }

    /// Push a vector of futures to run sequentially into the queue.
    ///
    /// This function submits the given future to the internal set for managing.
    /// This function will not call [`poll`](Future::poll) on the submitted
    /// future. The caller must ensure that [`FuturesUnorderedSeries::poll_next`] is
    /// called in order to receive task notifications.
    pub fn push_series(&mut self, mut futures: Vec<Fut>) {
        futures.reverse();
        if let Some(first) = futures.pop() {
            let wrapped = SeriesWrapper {
                data: first,
                index: self.next_index,
            };
            if !futures.is_empty() {
                self.series_queue.insert(self.next_index, futures);
            }
            self.next_index += 1;
            self.in_progress_queue.push(wrapped);
        }
    }
}

impl<Fut: Future> Default for FuturesUnorderedSeries<Fut> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Fut: Future> Stream for FuturesUnorderedSeries<Fut> {
    type Item = Fut::Output;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = &mut *self;

        // // Check to see if we've already received the next value
        // if let Some(next_output) = this.queued_outputs.peek_mut() {
        //     if next_output.index == this.next_outgoing_index {
        //         this.next_outgoing_index += 1;
        //         return Poll::Ready(Some(PeekMut::pop(next_output).data));
        //     }
        // }

        loop {
            match ready!(this.in_progress_queue.poll_next_unpin(cx)) {
                Some(output) => {
                    if let Some(futures) = this.series_queue.get_mut(&output.index) {
                        if let Some(next) = futures.pop() {
                            let wrapped = SeriesWrapper {
                                data: next,
                                index: output.index,
                            };
                            this.in_progress_queue.push(wrapped);
                        }
                        if futures.is_empty() {
                            this.series_queue.remove(&output.index);
                        }
                    }

                    return Poll::Ready(Some(output.data));
                }
                None => return Poll::Ready(None),
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<Fut: Future> Debug for FuturesUnorderedSeries<Fut> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FuturesUnorderedSeries {{ ... }}")
    }
}

impl<Fut: Future> FromIterator<Fut> for FuturesUnorderedSeries<Fut> {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = Fut>,
    {
        let acc = Self::new();
        iter.into_iter().fold(acc, |mut acc, item| {
            acc.push(item);
            acc
        })
    }
}

impl<Fut: Future> FusedStream for FuturesUnorderedSeries<Fut> {
    fn is_terminated(&self) -> bool {
        self.in_progress_queue.is_terminated() && self.series_queue.is_empty()
    }
}

impl<Fut: Future> Extend<Fut> for FuturesUnorderedSeries<Fut> {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Fut>,
    {
        for item in iter {
            self.push(item);
        }
    }
}
