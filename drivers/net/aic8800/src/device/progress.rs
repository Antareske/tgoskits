use alloc::vec::Vec;

use super::*;

#[cfg(test)]
const NANOS_PER_MILLISECOND: u64 = 1_000_000;

impl AicDevice {
    /// Begins initialization. No SDIO work is executed by this method.
    pub fn start(&mut self, now: MonotonicTime) -> Result<(), AicError> {
        if self.lifecycle.state != AicState::Stopped {
            return Err(AicError::Busy);
        }
        self.observe_time(now)?;
        self.lifecycle.state = AicState::Starting;
        self.lifecycle.startup = Some(StartupState::new());
        Ok(())
    }

    /// Advances at most one externally visible transition.
    pub fn advance(&mut self, input: AicInput) -> AicAction {
        if let Err(error) = self.observe_time(input.now) {
            return self.fail(error);
        }
        if let Some(event) = input.event
            && let Err(error) = self.consume_input(event, input.now)
        {
            return self.fail(error);
        }
        if let Some(event) = self.data.events.pop_front() {
            return AicAction::Event(event);
        }
        if self.lifecycle.cancel_pending
            && let Some(pending) = &self.io.pending
        {
            return AicAction::AbortSdio {
                request_id: pending.id,
            };
        }
        if self.io.pending.is_some() {
            return AicAction::WaitForInterrupt;
        }
        if let Some((purpose, kind)) = self.io.next.take() {
            return self.emit(purpose, kind);
        }
        if let Some(deadline) = self.lifecycle.retry_at {
            if input.now < deadline {
                return AicAction::RetryAt(deadline);
            }
            self.lifecycle.retry_at = None;
        }
        match self.lifecycle.state {
            AicState::Starting => self.drive_startup(input.now),
            AicState::Ready => self.drive_ready(input.now),
            AicState::Stopping => self.drive_shutdown(),
            AicState::Stopped | AicState::Failed => AicAction::Idle,
        }
    }

    fn observe_time(&mut self, now: MonotonicTime) -> Result<(), AicError> {
        if now < self.lifecycle.last_time {
            return Err(AicError::NonMonotonicTime);
        }
        self.lifecycle.last_time = now;
        Ok(())
    }

    fn consume_input(&mut self, event: AicInputEvent, now: MonotonicTime) -> Result<(), AicError> {
        match event {
            AicInputEvent::Sdio(completion) => self.consume_completion(completion, now),
            AicInputEvent::Irq(snapshot) => {
                if let Some(error) = snapshot.error {
                    return Err(AicError::Sdio(error));
                }
                if snapshot.sequence > self.io.last_irq_sequence {
                    self.io.last_irq_sequence = snapshot.sequence;
                    self.io.irq_pending |= snapshot.card_interrupt;
                }
                Ok(())
            }
            AicInputEvent::Control(request) => self.consume_control(request, now),
            AicInputEvent::Tx { token, frame } => {
                if self.lifecycle.state != AicState::Ready {
                    return Err(AicError::Busy);
                }
                self.data
                    .tx
                    .enqueue(token, frame)
                    .map_err(|_| AicError::TxQueueFull)
            }
        }
    }

    fn consume_control(
        &mut self,
        request: ControlRequest,
        _now: MonotonicTime,
    ) -> Result<(), AicError> {
        match request {
            ControlRequest::Cancel => {
                if self.lifecycle.control.is_none() && self.lifecycle.state != AicState::Starting {
                    return Err(AicError::InvalidControlRequest);
                }
                self.lifecycle.cancel_pending = true;
                if self.io.pending.is_none() {
                    self.finish_cancel();
                }
                Ok(())
            }
            ControlRequest::Shutdown => {
                self.lifecycle.state = AicState::Stopping;
                self.lifecycle.control = None;
                self.lifecycle.mailbox = None;
                self.lifecycle.cancel_pending = false;
                Ok(())
            }
            operation => {
                if self.lifecycle.state != AicState::Ready || self.lifecycle.control.is_some() {
                    return Err(AicError::Busy);
                }
                self.lifecycle.control =
                    Some(super::control::build(operation, self.data.mac_address)?);
                Ok(())
            }
        }
    }

    fn consume_completion(
        &mut self,
        completion: SdioCompletion,
        now: MonotonicTime,
    ) -> Result<(), AicError> {
        let pending = self.io.pending.take().ok_or(AicError::CompletionMismatch)?;
        if pending.id != completion.request_id {
            self.io.pending = Some(pending);
            return Err(AicError::CompletionMismatch);
        }
        if self.lifecycle.cancel_pending {
            if let Err(error) = completion.result
                && error != SdioFailure::Aborted
            {
                return Err(AicError::Sdio(error));
            }
            self.finish_cancel();
            return Ok(());
        }
        let response = completion.result.map_err(AicError::Sdio)?;
        match pending.purpose {
            IoPurpose::Startup => self.consume_startup_response(response, now),
            IoPurpose::MailboxFlow
            | IoPurpose::MailboxWrite
            | IoPurpose::MailboxCount
            | IoPurpose::MailboxRead => {
                self.consume_mailbox_response(pending.purpose, response, now)
            }
            IoPurpose::ReceiveCount => self.consume_receive_count(response),
            IoPurpose::ReceiveData => self.consume_receive_data(response),
            IoPurpose::TransmitFlow => self.consume_transmit_flow(response, now),
            IoPurpose::TransmitData => self.consume_transmit_data(response),
            IoPurpose::Shutdown => {
                expect_write_ack(response)?;
                self.lifecycle.state = AicState::Stopped;
                self.data.events.push_back(AicEvent::Stopped);
                Ok(())
            }
        }
    }

    pub(super) fn emit(&mut self, purpose: IoPurpose, kind: SdioRequestKind) -> AicAction {
        let id = self.io.next_request_id;
        self.io.next_request_id = self.io.next_request_id.wrapping_add(1).max(1);
        self.io.pending = Some(PendingIo { id, purpose });
        AicAction::SubmitSdio(SdioRequest { id, kind })
    }

    fn drive_shutdown(&mut self) -> AicAction {
        if let Some(pending) = &self.io.pending {
            return AicAction::AbortSdio {
                request_id: pending.id,
            };
        }
        self.data.rx.clear();
        let tokens: Vec<_> = self.data.tx.drain_tokens().collect();
        for token in tokens {
            self.data
                .events
                .push_back(AicEvent::TransmitComplete(token));
        }
        self.emit(
            IoPurpose::Shutdown,
            write_byte(1, self.registers.interrupt_enable, 0),
        )
    }

    fn finish_cancel(&mut self) {
        self.lifecycle.cancel_pending = false;
        self.lifecycle.mailbox = None;
        self.lifecycle.control = None;
        if self.lifecycle.state == AicState::Starting {
            self.lifecycle.startup = None;
            self.lifecycle.state = AicState::Stopped;
        }
        self.data.events.push_back(AicEvent::ControlCancelled);
    }

    pub(super) fn fail(&mut self, error: AicError) -> AicAction {
        self.lifecycle.state = AicState::Failed;
        self.io.pending = None;
        self.io.next = None;
        self.lifecycle.mailbox = None;
        self.lifecycle.control = None;
        if let Some(active) = self.data.active_tx.take() {
            self.data
                .events
                .push_back(AicEvent::TransmitComplete(active.token));
        }
        let tokens: Vec<_> = self.data.tx.drain_tokens().collect();
        self.data
            .events
            .extend(tokens.into_iter().map(AicEvent::TransmitComplete));
        self.data.events.push_back(AicEvent::Failed(error));
        AicAction::Event(
            self.data
                .events
                .pop_front()
                .expect("failure always publishes at least one terminal event"),
        )
    }

    pub(super) const fn command_function(&self) -> u8 {
        1
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::{
        common::{ChipVariant, SDIOWIFI_FUNC_BLOCKSIZE},
        device::startup::StartupStage,
    };

    fn time(ms: u64) -> MonotonicTime {
        MonotonicTime::from_nanos(ms * NANOS_PER_MILLISECOND)
    }

    fn complete(request: &SdioRequest, response: SdioResponse, now: MonotonicTime) -> AicInput {
        AicInput {
            now,
            event: Some(AicInputEvent::Sdio(SdioCompletion {
                request_id: request.id,
                result: Ok(response),
            })),
        }
    }

    #[test]
    fn startup_begins_with_protocol_owned_function_lifecycle() {
        let mut device = AicDevice::new(ChipVariant::Aic8801).unwrap();
        device.start(time(0)).unwrap();
        let AicAction::SubmitSdio(enable) = device.advance(AicInput::tick(time(0))) else {
            panic!("expected function enable")
        };
        assert!(matches!(
            enable.kind,
            SdioRequestKind::EnableFunction(number) if number.get() == 1
        ));
        let AicAction::SubmitSdio(block_size) =
            device.advance(complete(&enable, SdioResponse::Unit, time(0)))
        else {
            panic!("expected block-size configuration")
        };
        assert!(matches!(
            block_size.kind,
            SdioRequestKind::SetBlockSize { block_size, .. }
                if block_size.get() == SDIOWIFI_FUNC_BLOCKSIZE
        ));
    }

    /// Drives a D80 through the unit-completed stages up to the first
    /// vendor write so stage-injected tests share the same setup.
    fn first_vendor_write(device: &mut AicDevice) -> SdioRequest {
        device.start(time(0)).unwrap();
        let AicAction::SubmitSdio(enable) = device.advance(AicInput::tick(time(0))) else {
            panic!("expected function enable")
        };
        let AicAction::SubmitSdio(block_size) =
            device.advance(complete(&enable, SdioResponse::Unit, time(0)))
        else {
            panic!("expected block-size configuration")
        };
        let AicAction::SubmitSdio(interrupt) =
            device.advance(complete(&block_size, SdioResponse::Unit, time(0)))
        else {
            panic!("expected interrupt enable")
        };
        let AicAction::SubmitSdio(vendor) =
            device.advance(complete(&interrupt, SdioResponse::Unit, time(0)))
        else {
            panic!("expected vendor setup")
        };
        vendor
    }

    // The adapter completes every Direct operation (read or write) with the
    // R5 data byte, so the core's write consumers must accept a Byte
    // completion instead of rejecting it as malformed.

    #[test]
    fn vendor_setup_accepts_the_write_read_back_byte() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        let vendor = first_vendor_write(&mut device);
        assert!(matches!(vendor.kind, SdioRequestKind::WriteByte { .. }));
        let action = device.advance(complete(&vendor, SdioResponse::Byte(0x7f), time(0)));
        let AicAction::SubmitSdio(next) = action else {
            panic!("vendor write must consume the read-back byte, got {action:?}")
        };
        assert!(matches!(next.kind, SdioRequestKind::WriteByte { .. }));
        assert_ne!(device.state(), AicState::Failed);

        // The rejection side of the same contract: a Unit completion is not
        // the adapter shape for a Direct write.
        let mut rejected = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        let vendor = first_vendor_write(&mut rejected);
        let action = rejected.advance(complete(&vendor, SdioResponse::Unit, time(0)));
        assert!(matches!(action, AicAction::Event(AicEvent::Failed(_))));
    }

    #[test]
    fn reinitialize_accepts_the_write_read_back_byte() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        device.start(time(0)).unwrap();
        device.lifecycle.startup.as_mut().unwrap().stage = StartupStage::Reinitialize(0);
        let AicAction::SubmitSdio(request) = device.advance(AicInput::tick(time(0))) else {
            panic!("expected reinitialize write")
        };
        assert!(matches!(request.kind, SdioRequestKind::WriteByte { .. }));
        let action = device.advance(complete(&request, SdioResponse::Byte(0), time(0)));
        let AicAction::SubmitSdio(next) = action else {
            panic!("reinitialize must consume the read-back byte, got {action:?}")
        };
        assert!(matches!(next.kind, SdioRequestKind::WriteByte { .. }));
        assert_ne!(device.state(), AicState::Failed);

        let mut rejected = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        rejected.start(time(0)).unwrap();
        rejected.lifecycle.startup.as_mut().unwrap().stage = StartupStage::Reinitialize(0);
        let AicAction::SubmitSdio(request) = rejected.advance(AicInput::tick(time(0))) else {
            panic!("expected reinitialize write")
        };
        let action = rejected.advance(complete(&request, SdioResponse::Unit, time(0)));
        assert!(matches!(action, AicAction::Event(AicEvent::Failed(_))));
    }

    #[test]
    fn arm_chip_interrupt_accepts_the_write_read_back_byte() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        device.start(time(0)).unwrap();
        device.lifecycle.startup.as_mut().unwrap().stage = StartupStage::ArmChipInterrupt;
        let AicAction::SubmitSdio(request) = device.advance(AicInput::tick(time(0))) else {
            panic!("expected interrupt-enable write")
        };
        assert!(matches!(request.kind, SdioRequestKind::WriteByte { .. }));
        let action = device.advance(complete(&request, SdioResponse::Byte(0), time(0)));
        assert!(
            matches!(action, AicAction::Event(AicEvent::Started { .. })),
            "interrupt-enable write must finish startup, got {action:?}"
        );
        assert_eq!(device.state(), AicState::Ready);

        let mut rejected = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        rejected.start(time(0)).unwrap();
        rejected.lifecycle.startup.as_mut().unwrap().stage = StartupStage::ArmChipInterrupt;
        let AicAction::SubmitSdio(request) = rejected.advance(AicInput::tick(time(0))) else {
            panic!("expected interrupt-enable write")
        };
        let action = rejected.advance(complete(&request, SdioResponse::Unit, time(0)));
        assert!(matches!(action, AicAction::Event(AicEvent::Failed(_))));
    }

    #[test]
    fn shutdown_accepts_the_write_read_back_byte() {
        let mut device = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        device.start(time(0)).unwrap();
        // Drive the stop through the control path instead of forcing the
        // state, so the ControlRequest::Shutdown branch is exercised too.
        let action = device.advance(AicInput {
            now: time(0),
            event: Some(AicInputEvent::Control(ControlRequest::Shutdown)),
        });
        let AicAction::SubmitSdio(request) = action else {
            panic!("expected shutdown write")
        };
        assert!(matches!(request.kind, SdioRequestKind::WriteByte { .. }));
        let action = device.advance(complete(&request, SdioResponse::Byte(0), time(0)));
        assert!(
            matches!(action, AicAction::Event(AicEvent::Stopped)),
            "shutdown write must complete the stop, got {action:?}"
        );
        assert_eq!(device.state(), AicState::Stopped);

        let mut rejected = AicDevice::new(ChipVariant::Aic8800D80).unwrap();
        rejected.start(time(0)).unwrap();
        let action = rejected.advance(AicInput {
            now: time(0),
            event: Some(AicInputEvent::Control(ControlRequest::Shutdown)),
        });
        let AicAction::SubmitSdio(request) = action else {
            panic!("expected shutdown write")
        };
        let action = rejected.advance(complete(&request, SdioResponse::Unit, time(0)));
        assert!(matches!(action, AicAction::Event(AicEvent::Failed(_))));
    }

    #[test]
    fn retry_deadline_does_not_advance_early() {
        let mut device = AicDevice::new(ChipVariant::Aic8801).unwrap();
        device.start(time(0)).unwrap();
        device.lifecycle.retry_at = Some(time(10));
        assert_eq!(
            device.advance(AicInput::tick(time(9))),
            AicAction::RetryAt(time(10))
        );
    }

    #[test]
    fn cancellation_requests_abort_for_the_exact_active_transaction() {
        let mut device = AicDevice::new(ChipVariant::Aic8801).unwrap();
        device.start(time(0)).unwrap();
        let AicAction::SubmitSdio(request) = device.advance(AicInput::tick(time(0))) else {
            panic!("expected request")
        };
        let action = device.advance(AicInput {
            now: time(1),
            event: Some(AicInputEvent::Control(ControlRequest::Cancel)),
        });
        assert_eq!(
            action,
            AicAction::AbortSdio {
                request_id: request.id
            }
        );
    }

    #[test]
    fn aborted_completion_finishes_cancellation_without_failing_device() {
        let mut device = AicDevice::new(ChipVariant::Aic8801).unwrap();
        device.start(time(0)).unwrap();
        let AicAction::SubmitSdio(request) = device.advance(AicInput::tick(time(0))) else {
            panic!("expected request")
        };
        assert!(matches!(
            device.advance(AicInput {
                now: time(1),
                event: Some(AicInputEvent::Control(ControlRequest::Cancel)),
            }),
            AicAction::AbortSdio { .. }
        ));

        assert_eq!(
            device.advance(AicInput {
                now: time(1),
                event: Some(AicInputEvent::Sdio(SdioCompletion {
                    request_id: request.id,
                    result: Err(SdioFailure::Aborted),
                })),
            }),
            AicAction::Event(AicEvent::ControlCancelled)
        );
        assert_eq!(device.state(), AicState::Stopped);
    }

    #[test]
    fn non_monotonic_input_fails_closed() {
        let mut device = AicDevice::new(ChipVariant::Aic8801).unwrap();
        device.start(time(2)).unwrap();
        assert_eq!(
            device.advance(AicInput::tick(time(1))),
            AicAction::Event(AicEvent::Failed(AicError::NonMonotonicTime))
        );
    }

    #[test]
    fn failure_reclaims_active_and_queued_transmit_tokens_before_terminal_error() {
        let mut device = AicDevice::new(ChipVariant::Aic8801).unwrap();
        device.lifecycle.state = AicState::Ready;
        let active = TxToken::new(7);
        let queued = TxToken::new(8);
        device.data.active_tx = Some(ActiveTx {
            token: active,
            wire_frame: vec![1],
        });
        device.data.tx.enqueue(queued, vec![2]).unwrap();

        assert_eq!(
            device.fail(AicError::Sdio(SdioFailure::Bus)),
            AicAction::Event(AicEvent::TransmitComplete(active))
        );
        assert_eq!(
            device.advance(AicInput::tick(time(0))),
            AicAction::Event(AicEvent::TransmitComplete(queued))
        );
        assert_eq!(
            device.advance(AicInput::tick(time(0))),
            AicAction::Event(AicEvent::Failed(AicError::Sdio(SdioFailure::Bus)))
        );
    }
}
