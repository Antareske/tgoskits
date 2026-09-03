use crate::{
    common::{SDIOWIFI_REGISTER_BLOCK, SDIOWIFI_V3_WAKEUP_VALUE},
    device::*,
    registers::INTERRUPTS_ENABLED,
};

impl AicDevice {
    pub(super) fn vendor_setup_operation(
        &self,
        index: u8,
        reinitialize: bool,
    ) -> Option<SdioRequestKind> {
        if self.transport_generation() == crate::profile::TransportGeneration::V3 {
            match index {
                0 => Some(write_byte(0, 0xf2, 0x7f)),
                1 => Some(write_byte(1, self.registers().byte_mode_enable, 1)),
                2 => Some(write_byte(
                    1,
                    self.registers()
                        .wakeup
                        .expect("v3 chips define a wakeup register"),
                    SDIOWIFI_V3_WAKEUP_VALUE,
                )),
                3 if reinitialize => Some(write_byte(0, 0x04, INTERRUPTS_ENABLED)),
                _ => None,
            }
        } else {
            match index {
                0 => Some(write_byte(1, SDIOWIFI_REGISTER_BLOCK, 1)),
                1 => Some(write_byte(1, self.registers().byte_mode_enable, 1)),
                2 => Some(write_byte(2, SDIOWIFI_REGISTER_BLOCK, 1)),
                3 => Some(write_byte(2, self.registers().byte_mode_enable, 1)),
                4 => Some(write_byte(
                    1,
                    self.registers().interrupt_enable,
                    INTERRUPTS_ENABLED,
                )),
                5 => Some(write_byte(
                    2,
                    self.registers().interrupt_enable,
                    INTERRUPTS_ENABLED,
                )),
                _ => None,
            }
        }
    }

    pub(super) fn validate_vendor_setup_readback(
        &self,
        index: u8,
        reinitialize: bool,
        response: SdioResponse,
    ) -> Result<(), AicError> {
        let Some(SdioRequestKind::WriteByte { value, .. }) =
            self.vendor_setup_operation(index, reinitialize)
        else {
            return Err(AicError::CompletionMismatch);
        };
        // The V3 wakeup write (index 2) commands the chip out of light sleep, so its CMD52
        // RAW read-back byte reflects the register state the write is changing (0x01 while
        // the chip is still asleep) rather than the written value 0x11. The vendor driver
        // never compares this byte either; the write's effect is verified later by the
        // VendorReady sleep-status READY poll, so only the Byte shape is checked here.
        if self.transport_generation() == crate::profile::TransportGeneration::V3
            && index == 2
            && !reinitialize
        {
            expect_byte(response)?;
            return Ok(());
        }
        expect_write_readback(response, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ChipVariant;

    #[test]
    fn v3_wakeup_write_accepts_a_non_echoing_read_back_byte() {
        let device = AicDevice::new(ChipVariant::Aic8800D80).unwrap();

        assert_eq!(
            device.validate_vendor_setup_readback(2, false, SdioResponse::Byte(0x01)),
            Ok(())
        );
    }

    #[test]
    fn v3_vendor_writes_still_enforce_the_read_back_value() {
        let device = AicDevice::new(ChipVariant::Aic8800D80).unwrap();

        assert!(matches!(
            device.validate_vendor_setup_readback(0, false, SdioResponse::Byte(0x00)),
            Err(AicError::SdioWriteReadbackMismatch {
                expected: 0x7f,
                actual: 0x00
            })
        ));
    }
}
