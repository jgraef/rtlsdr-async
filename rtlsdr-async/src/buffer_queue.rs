use std::{
    collections::{
        HashMap,
        VecDeque,
    },
    fmt::Debug,
    ops::{
        Bound,
        DerefMut,
        RangeBounds,
    },
    sync::Arc,
    task::{
        Context,
        Poll,
        Waker,
    },
};

use futures_util::Stream;
use parking_lot::{
    Condvar,
    Mutex,
};

use crate::SampleType;

#[derive(Clone, derive_more::Debug)]
pub struct Buffer {
    #[debug(skip)]
    data: Arc<[u8]>,
    pub start: usize,
    pub end: usize,
    pub sample_rate: u32,
    pub sample_type: SampleType,
}

impl Buffer {
    pub(crate) fn new(capacity: usize) -> Self {
        let data = std::iter::repeat_n(0, capacity).collect();
        Self {
            data,
            start: 0,
            end: 0,
            sample_rate: 0,
            sample_type: SampleType::Iq,
        }
    }

    // note: this doesn't copy the buffer if we can't make it mut, but creates a new
    // one
    pub(crate) fn reclaim_or_allocate(&mut self, capacity: usize) -> &mut [u8] {
        if Arc::get_mut(&mut self.data).is_none() {
            tracing::debug!("Buffer::make_mut: creating new buffer");
            *self = Self::new(capacity);
        }

        Arc::get_mut(&mut self.data).expect("Arc::get_mut failed")
    }

    pub fn filled(&self) -> &[u8] {
        &self.data[self.start..self.end]
    }

    pub fn slice(&mut self, range: impl RangeBounds<usize>) {
        let start = match range.start_bound().cloned() {
            Bound::Included(start_bound) => self.start + start_bound,
            Bound::Excluded(start_bound) => self.start + start_bound + 1,
            Bound::Unbounded => self.start,
        };
        let end = match range.end_bound().cloned() {
            Bound::Included(end_bound) => self.start + end_bound + 1,
            Bound::Excluded(end_bound) => self.start + end_bound,
            Bound::Unbounded => self.end,
        };

        assert!(start >= self.start, "slice start out of bounds");
        assert!(start <= end, "slice start > end");
        assert!(end <= self.end, "slice end out of bounds");

        self.start = start;
        self.end = end;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug)]
struct Shared {
    state: Mutex<SharedState>,

    /// if there are no (active) receivers the reader thread will wait for
    /// this condition. in turn this should be notified, when the
    /// (active) receiver count becomes > 0, or if the subscriber and
    /// receiver count drops to 0. the latter is so the reader
    /// thread can resume, find out nobody is left, and terminate.
    receiver_count_changed: Condvar,
}

/// This is the central queue that passes buffers from the reader thread
/// (producer) to the AsyncReadSamples impl (consumer).
///
/// The items in the VecDeque are numbered head_pos..tail_pos from head to
/// tail. Consumers have a read_pos that is relative to that numbering,
/// so they'll know if they're lagging behind.
#[derive(Debug)]
struct SharedState {
    /// number of senders. this is either 1 or 0 currently.
    num_senders: usize,

    /// number of subscribers.
    /// these are like receivers, but they're not actively receiving data
    /// yet. so don't close the channel if there are subscribers or
    /// receivers. but we don't have to actually send anything to the
    /// channel if there are only subscribers.
    num_subscribers: usize,

    /// number of receivers
    num_receivers: usize,

    /// in-flight buffers
    slots: VecDeque<Buffer>,

    /// position where new buffers are appended
    tail_pos: usize,

    /// position of the oldest buffer. this corresponds to index 0 in
    /// `slots`
    head_pos: usize,

    /// total capacity for `slots`
    capacity: usize,

    /// wakers of receivers that are waiting for new buffers
    wakers: HashMap<usize, Waker>,

    /// receiver IDs to identify wakers with receivers.
    next_receiver_id: usize,
}

impl SharedState {
    fn pop_buffer(&mut self) -> Option<Buffer> {
        if self.slots.len() == self.capacity {
            let buffer = self
                .slots
                .pop_front()
                .expect("empty queue, but is at capacity");
            self.head_pos += 1;
            Some(buffer)
        }
        else {
            None
        }
    }

    fn push_buffer(&mut self, buffer: Buffer) {
        assert!(
            self.slots.len() < self.capacity,
            "expecting buffer queue to be below capacity when pushing"
        );
        self.slots.push_back(buffer);
        self.tail_pos += 1;
        for (_, waker) in self.wakers.drain() {
            waker.wake();
        }
    }
}

#[derive(Debug)]
pub struct Subscriber {
    shared: Arc<Shared>,
}

impl Clone for Subscriber {
    fn clone(&self) -> Self {
        let mut state = self.shared.state.lock();

        state.num_subscribers += 1;

        Self {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();

        state.num_subscribers -= 1;
    }
}

impl Subscriber {
    pub fn receiver(&self) -> Receiver {
        let mut state = self.shared.state.lock();

        state.num_receivers += 1;
        if state.num_receivers == 1 {
            self.shared.receiver_count_changed.notify_one();
        }
        let receiver_id = state.next_receiver_id;
        state.next_receiver_id += 1;

        Receiver {
            shared: self.shared.clone(),
            read_pos: state.tail_pos,
            receiver_id,
        }
    }
}

#[derive(Debug)]
pub struct Receiver {
    shared: Arc<Shared>,
    read_pos: usize,
    receiver_id: usize,
}

impl Clone for Receiver {
    fn clone(&self) -> Self {
        let mut state = self.shared.state.lock();

        state.num_receivers += 1;
        if state.num_receivers == 1 {
            self.shared.receiver_count_changed.notify_all();
        }
        let receiver_id = state.next_receiver_id;
        state.next_receiver_id += 1;

        Self {
            shared: self.shared.clone(),
            read_pos: self.read_pos,
            receiver_id,
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.num_receivers -= 1;
        if state.num_subscribers == 0 && state.num_receivers == 0 {
            self.shared.receiver_count_changed.notify_all();
        }
    }
}

impl Stream for Receiver {
    type Item = Buffer;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.deref_mut();

        let mut state = this.shared.state.lock();

        // determine index into the VecDeque
        let queue_index = if this.read_pos < state.head_pos {
            // we're behind, update our read_pos to the current head
            tracing::debug!(?this.read_pos, ?state.head_pos, ?state.tail_pos, "lagging behind by {} chunks", state.head_pos - this.read_pos);
            this.read_pos = state.head_pos;
            0
        }
        else {
            this.read_pos - state.head_pos
        };

        if this.read_pos < state.tail_pos {
            // there are buffers we can read
            let buffer = state.slots[queue_index].clone();
            this.read_pos += 1;
            Poll::Ready(Some(buffer))
        }
        else if state.num_senders == 0 {
            // there are no buffers left for us to read, and there are no writers left, so
            // we yield None
            Poll::Ready(None)
        }
        else {
            // there are no buffers left for us to read, but there are still writers, so we
            // need to wait.
            state.wakers.insert(this.receiver_id, cx.waker().clone());
            Poll::Pending
        }
    }
}

#[derive(derive_more::Debug)]
pub struct Sender {
    #[debug(skip)]
    shared: Arc<Shared>,
}

impl Drop for Sender {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.num_senders -= 1;
        if state.num_subscribers == 0 && state.num_receivers == 0 {
            self.shared.receiver_count_changed.notify_all();
        }
    }
}

impl Sender {
    /// Returns a buffer to be filled with data. You can also pass in a
    /// buffer that you just filled.
    ///
    /// Returns None if all receivers and subscribers dropped.
    ///
    /// If there are subscribers, but no receivers this will block until
    /// there is a receiver.
    ///
    /// # TODO
    ///
    /// Ideally we would like to have 3 methods:
    ///
    /// - block until receivers and swap
    /// - await until receivers and swap
    /// - just swap
    pub fn swap_buffers(
        &mut self,
        push_buffer: Option<Buffer>,
        buffer_size: usize,
        block: bool,
    ) -> Option<Buffer> {
        let mut state = self.shared.state.lock();

        while state.num_receivers == 0 && block {
            if state.num_subscribers == 0 {
                return None;
            }

            tracing::debug!("waiting for receivers");
            self.shared.receiver_count_changed.wait(&mut state);
            tracing::debug!(num_receivers = state.num_receivers, "resuming");
        }

        // first push the buffer we filled in the last loop iteration
        if let Some(buffer) = push_buffer {
            state.push_buffer(buffer);
        }

        // get a free buffer from the queue, or make a new one
        let buffer = state
            .pop_buffer()
            .unwrap_or_else(|| Buffer::new(buffer_size));

        Some(buffer)
    }
}

pub fn channel(num_buffers: usize) -> (Sender, Subscriber) {
    assert!(num_buffers > 0);

    let shared = Arc::new(Shared {
        state: Mutex::new(SharedState {
            num_subscribers: 1,
            num_receivers: 0,
            num_senders: 1,
            slots: VecDeque::with_capacity(num_buffers),
            tail_pos: 0,
            head_pos: 0,
            capacity: num_buffers,
            wakers: HashMap::new(),
            next_receiver_id: 0,
        }),
        receiver_count_changed: Condvar::new(),
    });

    (
        Sender {
            shared: shared.clone(),
        },
        Subscriber { shared },
    )
}
