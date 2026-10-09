//! Fragment validation, reassembly, and decompression for channel receives.

use alloc::{vec, vec::Vec};
use bevy_platform::collections::HashMap;

use crate::channel::receive::ChannelReceiveError;

type Result<T> = core::result::Result<T, ChannelReceiveError>;
#[cfg(feature = "compression_lz4")]
use crate::packet::compression::CompressionAlgorithm;
use crate::packet::compression::{CompressionConfig, decompress_payload};
use crate::packet::error::PacketError;
use crate::packet::message::{FragmentCompression, FragmentData, MessageId};
use crate::packet::packet::FRAGMENT_SIZE;
use bytes::Bytes;
use core::time::Duration;
use lightyear_core::tick::Tick;
use tracing::trace;

/// fallbeans patch: the largest message one fragmented message may claim. `FragmentConstructor::new`
/// allocates `num_fragments × fragment_size` up front from the peer's varint, so without a cap one
/// packet asks for any amount of memory. Fall Beans' largest message is the room list (16 rooms, ~9 KB).
pub const MAX_FRAGMENTED_MESSAGE_BYTES: usize = 64 * 1024;

/// fallbeans patch: what all partial messages of one channel may hold at once. A new message past it is
/// refused.
pub(crate) const MAX_PENDING_FRAGMENT_BYTES: usize = 1024 * 1024;

/// `FragmentReceiver` is used to reconstruct fragmented messages
#[derive(Debug)]
pub struct FragmentReceiver {
    fragment_messages: HashMap<MessageId, FragmentConstructor>,
    fragment_size: usize,
    /// fallbeans patch: bytes reserved by `fragment_messages`.
    pending_bytes: usize,
}

impl FragmentReceiver {
    pub fn new() -> Self {
        Self {
            fragment_messages: HashMap::default(),
            fragment_size: FRAGMENT_SIZE,
            pending_bytes: 0,
        }
    }

    pub(crate) fn set_fragment_size(&mut self, fragment_size: usize) {
        debug_assert!(fragment_size > 0);
        self.fragment_size = fragment_size;
    }

    /// Discard all messages for which the latest fragment was received before the cleanup time
    /// (i.e. we probably lost some fragments and we will never complete the message)
    ///
    /// If we don't keep track of the last received time, we will never clean up the messages.
    ///
    /// fallbeans patch: returns whether any message was dropped.
    pub fn cleanup(&mut self, cleanup_time: Duration) -> bool {
        let before = self.fragment_messages.len();
        let pending_bytes = &mut self.pending_bytes;
        self.fragment_messages.retain(|_, c| {
            let keep = c
                .last_received
                .map(|t| t > cleanup_time)
                .unwrap_or_else(|| true);
            if !keep {
                // fallbeans patch: give back what the dropped message reserved.
                *pending_bytes = pending_bytes.saturating_sub(c.reserved);
            }
            keep
        });
        self.fragment_messages.len() != before
    }

    /// Receive a fragment of a FragmentData message.
    ///
    /// When we complete the final message by aggregating all fragments, we will return the
    /// `remote_sent_tick` associated with the first fragment received.
    pub fn receive_fragment(
        &mut self,
        fragment: FragmentData,
        remote_sent_tick: Tick,
        current_time: Option<Duration>,
        compression: CompressionConfig,
    ) -> Result<Option<(Tick, Bytes)>> {
        let num_fragments = fragment.num_fragments.0;
        let fragment_index = fragment.fragment_id.0;
        if num_fragments == 0 {
            return Err(ChannelReceiveError::InvalidFragmentCount { num_fragments });
        }
        if fragment_index >= num_fragments {
            return Err(ChannelReceiveError::InvalidFragmentIndex {
                fragment_index,
                num_fragments,
            });
        }
        let num_fragments = usize::try_from(num_fragments)
            .map_err(|_| ChannelReceiveError::FragmentedMessageSizeOverflow)?;
        let fragment_index = usize::try_from(fragment_index)
            .map_err(|_| ChannelReceiveError::FragmentedMessageSizeOverflow)?;

        let message_id = fragment.message_id;
        let fragment_compression = fragment.compression;
        if !self.fragment_messages.contains_key(&message_id) {
            let constructor =
                FragmentConstructor::new(remote_sent_tick, num_fragments, self.fragment_size)?;
            // fallbeans patch: bound what the partial messages of the channel hold together.
            let pending_bytes = self
                .pending_bytes
                .checked_add(constructor.reserved)
                .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
            if pending_bytes > MAX_PENDING_FRAGMENT_BYTES {
                return Err(ChannelReceiveError::PendingFragmentsTooLarge {
                    pending: self.pending_bytes,
                    max: MAX_PENDING_FRAGMENT_BYTES,
                });
            }
            self.pending_bytes = pending_bytes;
            self.fragment_messages.insert(message_id, constructor);
        }
        let fragment_message = self
            .fragment_messages
            .get_mut(&message_id)
            .expect("fragment constructor was just inserted");
        if fragment_message.num_fragments != num_fragments {
            return Err(ChannelReceiveError::FragmentCountMismatch {
                expected: fragment_message.num_fragments,
                actual: num_fragments,
            });
        }
        if let Some(fragment_compression) = fragment_compression {
            fragment_message.set_compression(fragment_compression)?;
        } else if fragment_index == 0 {
            return Err(ChannelReceiveError::MissingFragmentCompression);
        }

        // completed the fragmented message!
        if let Some(payload) = fragment_message.receive_fragment(
            fragment_index,
            fragment.bytes.as_ref(),
            current_time,
        )? {
            let fragment_compression = fragment_message
                .compression
                .ok_or(ChannelReceiveError::MissingFragmentCompression)?;
            // fallbeans patch: give back what the completed message reserved.
            if let Some(done) = self.fragment_messages.remove(&message_id) {
                self.pending_bytes = self.pending_bytes.saturating_sub(done.reserved);
            }
            let (tick, payload) = payload;
            return decompress_fragment_payload(fragment_compression, payload, compression)
                .map(|payload| Some((tick, payload)));
        }

        Ok(None)
    }
}

#[derive(Debug, Clone)]
/// Data structure to reconstruct a single fragmented message from individual fragments
pub struct FragmentConstructor {
    num_fragments: usize,
    fragment_size: usize,
    num_received_fragments: usize,
    received: Vec<bool>,
    // bytes: Bytes,
    bytes: Vec<u8>,

    tick: Tick,
    compression: Option<FragmentCompression>,
    last_received: Option<Duration>,
    /// fallbeans patch: bytes counted against `MAX_PENDING_FRAGMENT_BYTES`.
    reserved: usize,
}

impl FragmentConstructor {
    pub fn new(tick: Tick, num_fragments: usize, fragment_size: usize) -> Result<Self> {
        let max_message_size = num_fragments
            .checked_mul(fragment_size)
            .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
        // fallbeans patch: refuse before allocating. The last fragment may be short, so the smallest
        // message with this many fragments is what must fit.
        let min_message_size = max_message_size
            .saturating_sub(fragment_size)
            .saturating_add(1);
        if min_message_size > MAX_FRAGMENTED_MESSAGE_BYTES {
            return Err(ChannelReceiveError::FragmentedMessageTooLarge {
                num_fragments,
                max: MAX_FRAGMENTED_MESSAGE_BYTES,
            });
        }
        Ok(Self {
            num_fragments,
            fragment_size,
            num_received_fragments: 0,
            received: vec![false; num_fragments],
            bytes: vec![0; max_message_size],
            tick,
            compression: None,
            last_received: None,
            reserved: max_message_size,
        })
    }

    pub fn set_compression(&mut self, compression: FragmentCompression) -> Result<()> {
        if let Some(expected) = self.compression {
            if expected != compression {
                return Err(ChannelReceiveError::FragmentCompressionMismatch {
                    expected: expected.as_str(),
                    actual: compression.as_str(),
                });
            }
        } else {
            self.compression = Some(compression);
        }
        Ok(())
    }

    pub fn receive_fragment(
        &mut self,
        fragment_index: usize,
        bytes: &[u8],
        received_time: Option<Duration>,
    ) -> Result<Option<(Tick, Bytes)>> {
        self.last_received = received_time;

        let is_last_fragment = fragment_index == self.num_fragments - 1;

        if !is_last_fragment && bytes.len() != self.fragment_size {
            return Err(ChannelReceiveError::InvalidNonFinalFragmentSize {
                actual: bytes.len(),
                expected: self.fragment_size,
            });
        }
        if is_last_fragment && bytes.len() > self.fragment_size {
            return Err(ChannelReceiveError::InvalidFinalFragmentSize {
                actual: bytes.len(),
                max: self.fragment_size,
            });
        }

        if !self.received[fragment_index] {
            self.received[fragment_index] = true;
            self.num_received_fragments += 1;

            if is_last_fragment {
                let len = (self.num_fragments - 1)
                    .checked_mul(self.fragment_size)
                    .and_then(|len| len.checked_add(bytes.len()))
                    .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
                self.bytes.resize(len, 0);
            }

            let start = fragment_index
                .checked_mul(self.fragment_size)
                .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
            let end = start
                .checked_add(bytes.len())
                .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
            self.bytes[start..end].copy_from_slice(bytes);
        }

        if self.num_received_fragments == self.num_fragments {
            trace!("Received all fragments!");
            let payload = core::mem::take(&mut self.bytes);
            return Ok(Some((self.tick, payload.into())));
        }

        Ok(None)
    }
}

fn decompress_fragment_payload(
    compression: FragmentCompression,
    payload: Bytes,
    config: CompressionConfig,
) -> Result<Bytes> {
    match compression {
        FragmentCompression::None => Ok(payload),
        FragmentCompression::Lz4 => {
            let config = config_for_fragment_compression(compression, config)?;
            decompress_payload(payload.as_ref(), config)
                .map(Bytes::from)
                .map_err(map_fragment_decompression_error)
        }
    }
}

fn config_for_fragment_compression(
    compression: FragmentCompression,
    config: CompressionConfig,
) -> Result<CompressionConfig> {
    match compression {
        FragmentCompression::None => Ok(config),
        FragmentCompression::Lz4 => {
            #[cfg(feature = "compression_lz4")]
            {
                if config.algorithm == Some(CompressionAlgorithm::Lz4) {
                    Ok(config)
                } else {
                    Err(ChannelReceiveError::UnsupportedFragmentCompression {
                        compression: compression.as_str(),
                    })
                }
            }
            #[cfg(not(feature = "compression_lz4"))]
            {
                Err(ChannelReceiveError::UnsupportedFragmentCompression {
                    compression: compression.as_str(),
                })
            }
        }
    }
}

fn map_fragment_decompression_error(error: PacketError) -> ChannelReceiveError {
    match error {
        PacketError::UnsupportedCompression => {
            ChannelReceiveError::UnsupportedFragmentCompression {
                compression: FragmentCompression::Lz4.as_str(),
            }
        }
        PacketError::DecompressionFailed => ChannelReceiveError::FragmentDecompressionFailed,
        PacketError::DecompressedPayloadTooLarge { actual, limit } => {
            ChannelReceiveError::FragmentDecompressedPayloadTooLarge { actual, limit }
        }
        _ => ChannelReceiveError::FragmentDecompressionFailed,
    }
}

#[cfg(test)]
mod tests {
    use crate::channel::fragments::FragmentSender;
    #[cfg(feature = "compression_lz4")]
    use crate::packet::compression::CompressionScratch;

    use super::*;

    #[test]
    fn test_receiver() -> Result<()> {
        let mut receiver = FragmentReceiver::new();
        let num_bytes = (FRAGMENT_SIZE as f32 * 1.5) as usize;
        let message_bytes = Bytes::from(vec![1u8; num_bytes]);
        let fragments = FragmentSender::new().build_fragments(MessageId(0), message_bytes.clone());

        assert_eq!(
            receiver.receive_fragment(
                fragments[1].clone(),
                Tick(1),
                None,
                CompressionConfig::DISABLED
            )?,
            None
        );
        assert_eq!(
            receiver.receive_fragment(
                fragments[0].clone(),
                Tick(0),
                None,
                CompressionConfig::DISABLED
            )?,
            Some((Tick(1), message_bytes.clone()))
        );
        Ok(())
    }

    #[test]
    fn reassembles_out_of_order_fragments_with_non_default_size() -> Result<()> {
        let mut sender = FragmentSender::new();
        sender.set_fragment_size(37);
        let message_bytes = Bytes::from(vec![3u8; 100]);
        let fragments = sender.build_fragments(MessageId(4), message_bytes.clone());
        let mut receiver = FragmentReceiver::new();
        receiver.set_fragment_size(37);

        assert_eq!(fragments.len(), 3);
        assert_eq!(
            receiver.receive_fragment(
                fragments[2].clone(),
                Tick(2),
                None,
                CompressionConfig::DISABLED,
            )?,
            None
        );
        assert_eq!(
            receiver.receive_fragment(
                fragments[0].clone(),
                Tick(0),
                None,
                CompressionConfig::DISABLED,
            )?,
            None
        );
        assert_eq!(
            receiver.receive_fragment(
                fragments[1].clone(),
                Tick(1),
                None,
                CompressionConfig::DISABLED,
            )?,
            Some((Tick(2), message_bytes))
        );
        Ok(())
    }

    #[cfg(feature = "compression_lz4")]
    #[test]
    fn compressed_fragments_are_decompressed_after_reassembly() -> Result<()> {
        let compression = CompressionConfig {
            min_payload_size: 0,
            max_decompressed_payload_size: FRAGMENT_SIZE * 4,
            ..CompressionConfig::LZ4
        };
        let mut receiver = FragmentReceiver::new();
        let message_bytes = Bytes::from(vec![7u8; FRAGMENT_SIZE * 3]);
        let fragments = FragmentSender::new().build_fragments_for_message(
            MessageId(1),
            message_bytes.clone(),
            compression,
            &mut CompressionScratch::default(),
        );

        assert!(fragments.len() < 3);
        assert_eq!(fragments[0].compression, Some(FragmentCompression::Lz4));
        assert!(
            fragments
                .iter()
                .skip(1)
                .all(|fragment| fragment.compression.is_none())
        );

        let mut result = None;
        for (index, fragment) in fragments.into_iter().enumerate() {
            result = receiver.receive_fragment(fragment, Tick(index as u32), None, compression)?;
        }

        assert_eq!(result, Some((Tick(0), message_bytes)));
        Ok(())
    }
}
