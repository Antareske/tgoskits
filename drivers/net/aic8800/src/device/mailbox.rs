use alloc::vec::Vec;
use core::time::Duration;

use super::*;
use crate::{
    protocol::{BLOCK_SIZE, command_frame, confirmation_payload, debug_command_frame},
    registers::{flow_credits, interrupt_block_count},
};

const MAILBOX_TIMEOUT: Duration = Duration::from_secs(5);
const MAILBOX_FLOW_RETRY: Duration = Duration::from_millis(1);
const MAILBOX_SETTLE: Duration = Duration::from_millis(2);
const MAX_MAILBOX_FLOW_RETRIES: u16 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MailboxPhase {
    Flow,
    Write,
    Settle,
    Count,
    Read { length: usize },
    Complete,
}

pub(super) struct MailboxState {
    frame: Vec<u8>,
    expected_message_id: u16,
    phase: MailboxPhase,
    deadline: MonotonicTime,
    retry_at: Option<MonotonicTime>,
    flow_retries: u16,
    result: Option<Vec<u8>>,
}

impl AicDevice {
    pub(super) fn drive_mailbox(&mut self, now: MonotonicTime) -> AicAction {
        let Some(mailbox) = self.lifecycle.mailbox.as_mut() else {
            return AicAction::Idle;
        };
        if now >= mailbox.deadline {
            return self.fail(AicError::MailboxTimeout);
        }
        if let Some(retry_at) = mailbox.retry_at {
            if now < retry_at {
                return AicAction::RetryAt(retry_at);
            }
            mailbox.retry_at = None;
        }
        match mailbox.phase {
            MailboxPhase::Flow => self.emit(
                IoPurpose::MailboxFlow,
                read_byte(1, self.registers.flow_control),
            ),
            MailboxPhase::Write => {
                let frame = mailbox.frame.clone();
                self.emit(
                    IoPurpose::MailboxWrite,
                    write_fifo(self.command_function(), self.registers.write_fifo, frame),
                )
            }
            MailboxPhase::Settle => {
                mailbox.phase = MailboxPhase::Count;
                self.drive_mailbox(now)
            }
            MailboxPhase::Count => self.emit(
                IoPurpose::MailboxCount,
                read_byte(self.command_function(), self.registers.block_count),
            ),
            MailboxPhase::Read { length } => self.emit(
                IoPurpose::MailboxRead,
                read_fifo(self.command_function(), self.registers.read_fifo, length),
            ),
            MailboxPhase::Complete => match self.complete_mailbox() {
                Ok(()) => self.drive_startup_or_ready(now),
                Err(error) => self.fail(error),
            },
        }
    }

    pub(super) fn consume_mailbox_response(
        &mut self,
        purpose: IoPurpose,
        response: SdioResponse,
        now: MonotonicTime,
    ) -> Result<(), AicError> {
        let mailbox = self
            .lifecycle
            .mailbox
            .as_mut()
            .ok_or(AicError::CompletionMismatch)?;
        match purpose {
            IoPurpose::MailboxFlow => {
                let flow = flow_credits(expect_byte(response)?);
                if flow == 0 {
                    mailbox.flow_retries = mailbox.flow_retries.saturating_add(1);
                    if mailbox.flow_retries >= MAX_MAILBOX_FLOW_RETRIES {
                        return Err(AicError::MailboxTimeout);
                    }
                    mailbox.retry_at = Some(now.after(MAILBOX_FLOW_RETRY));
                } else {
                    mailbox.phase = MailboxPhase::Write;
                }
            }
            IoPurpose::MailboxWrite => {
                expect_unit(response)?;
                mailbox.phase = MailboxPhase::Settle;
                mailbox.retry_at = Some(now.after(MAILBOX_SETTLE));
            }
            IoPurpose::MailboxCount => match interrupt_block_count(expect_byte(response)?) {
                None | Some(0) => {
                    mailbox.retry_at = Some(now.after(MAILBOX_FLOW_RETRY));
                }
                Some(count) => {
                    mailbox.phase = MailboxPhase::Read {
                        length: usize::from(count) * BLOCK_SIZE,
                    };
                }
            },
            IoPurpose::MailboxRead => {
                let data = expect_data(response)?;
                mailbox.result = Some(
                    confirmation_payload(&data, mailbox.expected_message_id)
                        .map_err(|_| AicError::MalformedResponse)?,
                );
                mailbox.phase = MailboxPhase::Complete;
            }
            _ => return Err(AicError::CompletionMismatch),
        }
        Ok(())
    }

    fn complete_mailbox(&mut self) -> Result<(), AicError> {
        let result = self
            .lifecycle
            .mailbox
            .take()
            .and_then(|mut mailbox| mailbox.result.take())
            .ok_or(AicError::MalformedResponse)?;
        if self.lifecycle.state == AicState::Starting {
            self.complete_startup_mailbox(result)
        } else if let Some(control) = self.lifecycle.control.as_mut() {
            control.commands.pop_front();
            if control.commands.is_empty() {
                self.lifecycle.control = None;
                self.data.events.push_back(AicEvent::ControlComplete);
            }
            Ok(())
        } else {
            Err(AicError::CompletionMismatch)
        }
    }

    fn drive_startup_or_ready(&mut self, now: MonotonicTime) -> AicAction {
        if self.lifecycle.state == AicState::Starting {
            self.drive_startup(now)
        } else {
            self.drive_ready(now)
        }
    }

    pub(super) fn begin_debug_mailbox(
        &mut self,
        message_id: u16,
        payload: &[u8],
        now: MonotonicTime,
    ) {
        self.lifecycle.mailbox = Some(MailboxState {
            frame: debug_command_frame(message_id, payload, self.chip.is_v3()),
            expected_message_id: message_id + 1,
            phase: MailboxPhase::Flow,
            deadline: now.after(MAILBOX_TIMEOUT),
            retry_at: None,
            flow_retries: 0,
            result: None,
        });
    }

    pub(super) fn begin_lmac_mailbox(
        &mut self,
        message_id: u16,
        destination: u16,
        payload: &[u8],
        expected_message_id: u16,
        now: MonotonicTime,
    ) {
        self.lifecycle.mailbox = Some(MailboxState {
            frame: command_frame(message_id, destination, payload, self.chip.is_v3()),
            expected_message_id,
            phase: MailboxPhase::Flow,
            deadline: now.after(MAILBOX_TIMEOUT),
            retry_at: None,
            flow_retries: 0,
            result: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::{
        AicInputEvent, ChipVariant, SdioCompletion, SdioRequestKind, SdioResponse,
        device::startup::StartupStage, protocol::TASK_MM,
    };

    const NANOS_PER_MILLISECOND: u64 = 1_000_000;

    fn time(ms: u64) -> MonotonicTime {
        MonotonicTime::from_nanos(ms * NANOS_PER_MILLISECOND)
    }

    fn completion(request_id: u64, response: SdioResponse, now: MonotonicTime) -> AicInput {
        AicInput {
            now,
            event: Some(AicInputEvent::Sdio(SdioCompletion {
                request_id,
                result: Ok(response),
            })),
        }
    }

    /// Drives one mailbox through flow control, the frame write, the settle
    /// retry and the count poll, and returns the pending read request.
    fn pending_read(device: &mut AicDevice, first: SdioRequest) -> SdioRequest {
        let action = device.advance(completion(first.id, SdioResponse::Byte(0x7f), time(2)));
        let AicAction::SubmitSdio(write) = action else {
            panic!("expected mailbox frame write")
        };
        let action = device.advance(completion(write.id, SdioResponse::Unit, time(3)));
        assert!(matches!(action, AicAction::RetryAt(_)));
        let action = device.advance(AicInput::tick(time(5)));
        let AicAction::SubmitSdio(count) = action else {
            panic!("expected mailbox count poll")
        };
        let action = device.advance(completion(count.id, SdioResponse::Byte(1), time(6)));
        let AicAction::SubmitSdio(read) = action else {
            panic!("expected mailbox response read")
        };
        read
    }

    fn confirmation_frame(message_id: u16, payload: &[u8]) -> alloc::vec::Vec<u8> {
        let mut frame = vec![0u8; BLOCK_SIZE];
        frame[4..6].copy_from_slice(&message_id.to_le_bytes());
        frame[10..12].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        frame[16..16 + payload.len()].copy_from_slice(payload);
        frame
    }

    #[test]
    fn read_revision_takes_the_version_from_the_high_half_of_the_read_back() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).expect("D80 is supported");
        device.start(time(0)).expect("stopped device starts");
        device.lifecycle.startup.as_mut().unwrap().stage = StartupStage::ReadRevision;
        device.begin_debug_mailbox(0x0400, &[0; 4], time(0));
        let AicAction::SubmitSdio(first) = device.advance(AicInput::tick(time(0))) else {
            panic!("mailbox must produce a first SDIO request");
        };
        let read = pending_read(&mut device, first);

        // The chip reports its revision in the high half of the read-back
        // word (the low half is unrelated value noise, as seen on the board).
        let mut payload = [0u8; 8];
        payload[4..8].copy_from_slice(&0x0001_0020u32.to_le_bytes()); // rev U01, noise 0x20
        let action = device.advance(completion(
            read.id,
            SdioResponse::Data(confirmation_frame(0x0401, &payload)),
            time(7),
        ));
        let AicAction::SubmitSdio(upload) = action else {
            panic!("revision U01 must be accepted, got {action:?}")
        };
        assert_eq!(
            device.lifecycle.startup.as_ref().unwrap().stage,
            StartupStage::UploadMain(0)
        );
        assert!(matches!(
            upload.kind,
            SdioRequestKind::ReadByte { address, .. } if address.get() == 3 // flow control
        ));
        assert_ne!(device.state(), AicState::Failed);
    }

    #[test]
    fn get_mac_address_confirmation_installs_the_mac_and_arms_the_chip_interrupt() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).expect("D80 is supported");
        device.start(time(0)).expect("stopped device starts");
        device.lifecycle.startup.as_mut().unwrap().stage = StartupStage::GetMacAddress;
        device.begin_lmac_mailbox(0x0073, TASK_MM, &1u32.to_le_bytes(), 0x0074, time(0));
        let AicAction::SubmitSdio(first) = device.advance(AicInput::tick(time(0))) else {
            panic!("mailbox must produce a first SDIO request");
        };
        let read = pending_read(&mut device, first);

        let mac = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let action = device.advance(completion(
            read.id,
            SdioResponse::Data(confirmation_frame(0x0074, &mac)),
            time(7),
        ));
        assert_eq!(device.mac_address(), mac);
        assert_eq!(
            device.lifecycle.startup.as_ref().unwrap().stage,
            StartupStage::ArmChipInterrupt
        );
        let AicAction::SubmitSdio(arm) = action else {
            panic!("startup must continue with the interrupt-enable write, got {action:?}")
        };
        assert!(matches!(arm.kind, SdioRequestKind::WriteByte { .. }));
        assert_ne!(device.state(), AicState::Failed);
    }

    #[test]
    fn get_mac_address_confirmation_shorter_than_the_address_fails_the_device() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).expect("D80 is supported");
        device.start(time(0)).expect("stopped device starts");
        device.lifecycle.startup.as_mut().unwrap().stage = StartupStage::GetMacAddress;
        device.begin_lmac_mailbox(0x0073, TASK_MM, &1u32.to_le_bytes(), 0x0074, time(0));
        let AicAction::SubmitSdio(first) = device.advance(AicInput::tick(time(0))) else {
            panic!("mailbox must produce a first SDIO request");
        };
        let read = pending_read(&mut device, first);

        assert!(
            matches!(
                device.advance(completion(
                    read.id,
                    SdioResponse::Data(confirmation_frame(0x0074, &[0x02, 0x03, 0x04, 0x05, 0x06])),
                    time(7),
                )),
                AicAction::Event(AicEvent::Failed(AicError::MalformedResponse))
            ),
            "a truncated get-mac confirmation must fail the device"
        );
    }
}
