//! The fixed-size buffer between a capture thread and its track writer.
//!
//! Its memory is allocated once and never grows. When the writer falls behind and the buffer
//! is full, the capture thread does not wait and does not allocate: the packet is dropped,
//! the dropped frames and the time they covered are counted, and capture carries on.
//!
//! The writer always takes everything at once, so the "ring" is two buffers of the same fixed
//! size that trade places: one being filled, one being written out.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// What the capture thread knows about one packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketMeta {
    /// Timeline time of the packet's first frame.
    pub timeline_us: i64,
    pub frames: u32,
    /// Clock map flags (see [`crate::clock::flags`]).
    pub flags: u32,
}

/// What the writer takes out, in the order it went in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Item {
    /// A packet. Its bytes are next in the drained byte buffer.
    Packet(PacketMeta),
    /// One or more packets that did not fit.
    Dropped {
        frames: u64,
        from_us: i64,
        until_us: i64,
    },
}

struct State {
    bytes: Vec<u8>,
    items: Vec<Item>,
    packets: usize,
    producer_closed: bool,
    consumer_closed: bool,
}

struct Shared {
    capacity_bytes: usize,
    max_packets: usize,
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) struct Producer {
    shared: Arc<Shared>,
}

/// The writer's end. It owns a second buffer of the same fixed size and trades it for the
/// filled one, so taking the audio out is a swap, not a copy, and the capture thread is held
/// up for no longer than that.
pub(crate) struct Consumer {
    shared: Arc<Shared>,
    bytes: Vec<u8>,
    items: Vec<Item>,
}

/// A ring holding at most `capacity_bytes` of audio in at most `max_packets` packets.
pub(crate) fn ring(capacity_bytes: usize, max_packets: usize) -> (Producer, Consumer) {
    // One slot more than the packets, for a marker of dropped packets behind them.
    let item_capacity = max_packets + 1;
    let shared = Arc::new(Shared {
        capacity_bytes,
        max_packets,
        state: Mutex::new(State {
            bytes: Vec::with_capacity(capacity_bytes),
            items: Vec::with_capacity(item_capacity),
            packets: 0,
            producer_closed: false,
            consumer_closed: false,
        }),
        changed: Condvar::new(),
    });
    (
        Producer {
            shared: Arc::clone(&shared),
        },
        Consumer {
            shared,
            bytes: Vec::with_capacity(capacity_bytes),
            items: Vec::with_capacity(item_capacity),
        },
    )
}

/// What became of a packet offered to the ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pushed {
    Stored,
    /// The ring was full. The frames are counted and reported to the writer.
    Dropped,
    /// The writer has gone; nothing is counted because nothing can record it.
    Closed,
}

impl Producer {
    /// Offers a packet. Never blocks beyond the short copy and never allocates: both checks
    /// below keep the buffers within the capacity they were created with.
    pub(crate) fn push(&self, meta: PacketMeta, data: &[u8], duration_us: i64) -> Pushed {
        let mut state = self.shared.lock();
        if state.consumer_closed || state.producer_closed {
            return Pushed::Closed;
        }
        let full = state.bytes.len() + data.len() > self.shared.capacity_bytes
            || state.packets >= self.shared.max_packets;
        let pushed = if full {
            let until_us = meta.timeline_us + duration_us;
            match state.items.last_mut() {
                Some(Item::Dropped {
                    frames,
                    until_us: last,
                    ..
                }) => {
                    *frames += u64::from(meta.frames);
                    *last = until_us;
                }
                _ => state.items.push(Item::Dropped {
                    frames: u64::from(meta.frames),
                    from_us: meta.timeline_us,
                    until_us,
                }),
            }
            Pushed::Dropped
        } else {
            state.bytes.extend_from_slice(data);
            state.packets += 1;
            state.items.push(Item::Packet(meta));
            Pushed::Stored
        };
        drop(state);
        self.shared.changed.notify_one();
        pushed
    }

    /// No more packets will come. The writer drains what is left and finishes.
    pub(crate) fn close(&self) {
        self.shared.lock().producer_closed = true;
        self.shared.changed.notify_one();
    }
}

/// Everything that was in the ring, in order.
pub(crate) struct Drained<'a> {
    /// The bytes of every packet in `items`, back to back.
    pub bytes: &'a [u8],
    pub items: &'a [Item],
    /// `false` once the producer has closed and the ring is empty.
    pub open: bool,
}

impl Consumer {
    /// Takes everything in the ring, waiting up to `wait` for something to arrive.
    pub(crate) fn drain(&mut self, wait: Duration) -> Drained<'_> {
        self.bytes.clear();
        self.items.clear();
        let mut state = self.shared.lock();
        if state.items.is_empty() && !state.producer_closed {
            state = self
                .shared
                .changed
                .wait_timeout(state, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        std::mem::swap(&mut self.bytes, &mut state.bytes);
        std::mem::swap(&mut self.items, &mut state.items);
        state.packets = 0;
        let open = !(self.items.is_empty() && state.producer_closed);
        drop(state);
        Drained {
            bytes: &self.bytes,
            items: &self.items,
            open,
        }
    }

    /// The writer can take no more. Later packets are refused instead of filling the ring.
    pub(crate) fn close(&self) {
        self.shared.lock().consumer_closed = true;
    }

    /// The most audio the ring, and so each drain, can hold.
    #[cfg(test)]
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.shared.capacity_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_millis(1);

    fn meta(timeline_us: i64, frames: u32) -> PacketMeta {
        PacketMeta {
            timeline_us,
            frames,
            flags: 0,
        }
    }

    fn drain(consumer: &mut Consumer) -> (Vec<u8>, Vec<Item>, bool) {
        let drained = consumer.drain(WAIT);
        (drained.bytes.to_vec(), drained.items.to_vec(), drained.open)
    }

    #[test]
    fn packets_come_out_in_order_with_their_bytes() {
        let (producer, mut consumer) = ring(16, 8);
        assert_eq!(producer.push(meta(0, 2), &[1, 2, 3, 4], 10), Pushed::Stored);
        assert_eq!(producer.push(meta(10, 1), &[5, 6], 5), Pushed::Stored);

        let (bytes, items, open) = drain(&mut consumer);

        assert_eq!(bytes, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(
            items,
            vec![Item::Packet(meta(0, 2)), Item::Packet(meta(10, 1))]
        );
        assert!(open);
    }

    #[test]
    fn a_full_ring_drops_the_packet_counts_it_and_keeps_going() {
        let (producer, mut consumer) = ring(8, 8);
        assert_eq!(producer.push(meta(0, 4), &[1; 8], 100), Pushed::Stored);
        assert_eq!(producer.push(meta(100, 4), &[2; 8], 100), Pushed::Dropped);
        assert_eq!(producer.push(meta(200, 2), &[3; 4], 50), Pushed::Dropped);

        let (bytes, items, _) = drain(&mut consumer);
        assert_eq!(bytes, vec![1; 8]);
        assert_eq!(
            items,
            vec![
                Item::Packet(meta(0, 4)),
                Item::Dropped {
                    frames: 6,
                    from_us: 100,
                    until_us: 250
                }
            ]
        );

        // There is room again, so capture continues after the gap.
        assert_eq!(producer.push(meta(250, 4), &[4; 8], 100), Pushed::Stored);
        let (bytes, items, _) = drain(&mut consumer);
        assert_eq!(bytes, vec![4; 8]);
        assert_eq!(items, vec![Item::Packet(meta(250, 4))]);
    }

    #[test]
    fn the_packet_limit_bounds_the_queue_as_well_as_the_bytes() {
        let (producer, mut consumer) = ring(1_024, 2);
        assert_eq!(producer.push(meta(0, 1), &[1], 1), Pushed::Stored);
        assert_eq!(producer.push(meta(1, 1), &[2], 1), Pushed::Stored);
        assert_eq!(producer.push(meta(2, 1), &[3], 1), Pushed::Dropped);
        assert_eq!(producer.push(meta(3, 1), &[4], 1), Pushed::Dropped);

        let (bytes, items, _) = drain(&mut consumer);

        assert_eq!(bytes, vec![1, 2]);
        assert_eq!(items.len(), 3, "two packets and one merged drop marker");
    }

    #[test]
    fn its_memory_never_grows_however_much_is_offered() {
        let (producer, mut consumer) = ring(64, 4);
        let capacity = |consumer: &Consumer| {
            let state = consumer.shared.lock();
            (
                state.bytes.capacity(),
                state.items.capacity(),
                consumer.bytes.capacity(),
                consumer.items.capacity(),
            )
        };
        let before = capacity(&consumer);

        for round in 0..200 {
            for packet in 0..10 {
                producer.push(meta(round * 10 + packet, 4), &[7; 24], 1);
            }
            let drained = consumer.drain(WAIT);
            assert!(drained.bytes.len() <= 64);
            assert!(drained.items.len() <= 5);
        }

        assert_eq!(capacity(&consumer), before);
        assert_eq!(consumer.capacity_bytes(), 64);
    }

    #[test]
    fn closing_lets_the_writer_drain_what_is_left_then_stop() {
        let (producer, mut consumer) = ring(16, 8);
        producer.push(meta(0, 1), &[1, 2], 1);
        producer.close();
        assert_eq!(producer.push(meta(1, 1), &[3, 4], 1), Pushed::Closed);

        let (bytes, _, open) = drain(&mut consumer);
        assert_eq!(bytes, vec![1, 2]);
        assert!(open, "the last packets are still delivered");
        let (bytes, items, open) = drain(&mut consumer);
        assert!(bytes.is_empty() && items.is_empty());
        assert!(!open);
    }

    #[test]
    fn a_writer_that_has_gone_refuses_packets_without_counting_them() {
        let (producer, consumer) = ring(16, 8);
        consumer.close();

        assert_eq!(producer.push(meta(0, 1), &[1, 2], 1), Pushed::Closed);
    }

    #[test]
    fn a_waiting_writer_wakes_when_a_packet_arrives() {
        let (producer, mut consumer) = ring(16, 8);
        let writer =
            std::thread::spawn(move || consumer.drain(Duration::from_secs(30)).bytes.to_vec());
        std::thread::sleep(Duration::from_millis(20));
        producer.push(meta(0, 1), &[9, 9], 1);

        assert_eq!(writer.join().expect("writer"), vec![9, 9]);
    }
}
