use std::{
    sync::Arc,
    thread,
};

use crate::{
    Error,
    buffer_queue::{
        self,
        Buffer,
    },
    handle::Handle,
};

pub(crate) fn spawn_reader_thread(
    handle: Arc<Handle>,
    buffer_size: usize,
    queue_size: usize,
) -> buffer_queue::Subscriber {
    let (buffer_queue_sender, buffer_queue_subscriber) = buffer_queue::channel(queue_size);

    thread::spawn({
        let handle = handle.clone();
        move || {
            reader_thread(buffer_queue_sender, handle, buffer_size);
        }
    });

    buffer_queue_subscriber
}

fn reader_thread(
    mut buffer_queue_sender: buffer_queue::Sender,
    handle: Arc<Handle>,
    buffer_size: usize,
) {
    let _guard = tracing::debug_span!("reader thread", index = handle.index).entered();

    // when we are reading to the buffer we don't hold the queue lock, so once we're
    // done we need to acquire the lock to add the buffer to the queue.
    // but we also need the queue lock to get a new free buffer. we can combine both
    // steps into one lock-holding code section at the start of the loop. All we
    // need to do is remember the buffer we want to push.
    let mut push_buffer = None;

    tracing::debug!("reader thread spawned");

    loop {
        let Some(mut buffer) =
            buffer_queue_sender.swap_buffers(push_buffer.take(), buffer_size, true)
        else {
            // all receivers and subscribers dropped
            tracing::debug!("all readers dropped. exiting");
            break;
        };

        match read_to_buffer(&handle, &mut buffer, buffer_size) {
            Ok(true) => {
                push_buffer = Some(buffer);
            }
            Ok(false) => {
                tracing::debug!("rtlsdr_read_sync returned 0. exiting");
                break;
            }
            Err(error) => {
                tracing::error!(?error, "rtlsdr reader thread error");
                break;
            }
        }
    }
}

fn read_to_buffer(handle: &Handle, buffer: &mut Buffer, buffer_size: usize) -> Result<bool, Error> {
    let mut handle = handle.lock();

    buffer.sample_rate = handle.get_sample_rate()?;
    buffer.sample_type = handle.get_direct_sampling()?.into();

    // this will try to reclaim the buffer. if it can't, it'll create a new one.
    let buffer_mut = buffer.reclaim_or_allocate(buffer_size);

    // note: we could call read_sync multiple times if one call doesn't fill the
    // buffer, but testing shows that it usually fills the buffer.
    // not sure how it will behave with larger buffer sizes, but you should then
    // probably choose a better buffer size.
    let n_read = handle.read_sync(buffer_mut)?;

    if n_read > 0 {
        assert!(
            n_read & 1 == 0,
            "not an even amount of bytes ({n_read}) :sobbing: open an issue and i will fix this, but i thought this would never happen",
        );
        buffer.start = 0;
        buffer.end = n_read;
        Ok(true)
    }
    else {
        tracing::debug!("rtlsdr_read_sync returned 0. exiting");
        Ok(false)
    }
}
