//! Typed AIC LMAC message construction and confirmation parsing.
//!
//! Layouts here follow the vendor Linux AIC8800 driver. Device ownership and
//! state transitions deliberately live outside this wire-format module.

use alloc::{vec, vec::Vec};

use crate::{common::ChipVariant, device::AicError};

pub(crate) const TASK_MM: u16 = 0;
pub(crate) const TASK_ME: u16 = 5;
pub(crate) const TASK_SM: u16 = 6;

pub(crate) const MM_RESET_REQ: u16 = 0x0000;
pub(crate) const MM_RESET_CFM: u16 = 0x0001;
pub(crate) const MM_START_REQ: u16 = 0x0002;
pub(crate) const MM_START_CFM: u16 = 0x0003;
pub(crate) const MM_ADD_IF_REQ: u16 = 0x0006;
pub(crate) const MM_ADD_IF_CFM: u16 = 0x0007;
pub(crate) const MM_SET_FILTER_REQ: u16 = 0x000e;
pub(crate) const MM_SET_FILTER_CFM: u16 = 0x000f;
pub(crate) const APM_START_REQ: u16 = 0x1c00;
pub(crate) const APM_START_CFM: u16 = 0x1c01;
pub(crate) const APM_SET_BEACON_IE_REQ: u16 = 0x1c08;
pub(crate) const APM_SET_BEACON_IE_CFM: u16 = 0x1c09;
pub(crate) const MM_KEY_ADD_REQ: u16 = 0x0024;
pub(crate) const MM_KEY_ADD_CFM: u16 = 0x0025;
pub(crate) const MM_SET_RF_CALIB_REQ: u16 = 0x0069;
pub(crate) const MM_SET_RF_CALIB_CFM: u16 = 0x006a;
pub(crate) const MM_SET_RF_CONFIG_REQ: u16 = 0x0067;
pub(crate) const MM_SET_RF_CONFIG_CFM: u16 = 0x0068;
pub(crate) const MM_GET_MAC_ADDR_REQ: u16 = 0x0073;
pub(crate) const MM_GET_MAC_ADDR_CFM: u16 = 0x0074;
pub(crate) const MM_SET_STACK_START_REQ: u16 = 0x007b;
pub(crate) const MM_SET_STACK_START_CFM: u16 = 0x007c;
// Unsolicited MM indications may be interleaved with control confirmations
// while the firmware is associating.  They share the CFG_CMD_RSP transport
// type, so the receive parser needs the protocol classification rather than
// treating every non-SM message as a mailbox confirmation.
pub(crate) const MM_PRIMARY_TBTT_IND: u16 = 0x002c;
pub(crate) const MM_SECONDARY_TBTT_IND: u16 = 0x002d;
pub(crate) const MM_CONNECTION_LOSS_IND: u16 = 0x0043;
pub(crate) const MM_CHANNEL_SWITCH_IND: u16 = 0x0044;
pub(crate) const MM_CHANNEL_PRE_SWITCH_IND: u16 = 0x0045;
pub(crate) const MM_REMAIN_ON_CHANNEL_EXP_IND: u16 = 0x0048;
pub(crate) const MM_PS_CHANGE_IND: u16 = 0x0049;
pub(crate) const MM_TRAFFIC_REQ_IND: u16 = 0x004a;
pub(crate) const MM_P2P_VIF_PS_CHANGE_IND: u16 = 0x004d;
pub(crate) const MM_CSA_COUNTER_IND: u16 = 0x004e;
pub(crate) const MM_CHANNEL_SURVEY_IND: u16 = 0x004f;
pub(crate) const MM_P2P_NOA_UPD_IND: u16 = 0x0055;
pub(crate) const MM_RSSI_STATUS_IND: u16 = 0x0057;
pub(crate) const MM_CSA_FINISH_IND: u16 = 0x0058;
pub(crate) const MM_CSA_TRAFFIC_IND: u16 = 0x0059;
pub(crate) const MM_PKTLOSS_IND: u16 = 0x0060;
pub(crate) const MM_APM_STALOSS_IND: u16 = 0x007d;
pub(crate) const MM_RADAR_DETECT_IND: u16 = 0x008b;
pub(crate) const MM_SET_TXPWR_IDX_LVL_REQ: u16 = 0x0077;
pub(crate) const MM_SET_TXPWR_IDX_LVL_CFM: u16 = 0x0078;
pub(crate) const ME_CONFIG_REQ: u16 = 0x1400;
pub(crate) const ME_CONFIG_CFM: u16 = 0x1401;
pub(crate) const ME_CHAN_CONFIG_REQ: u16 = 0x1402;
pub(crate) const ME_CHAN_CONFIG_CFM: u16 = 0x1403;
pub(crate) const ME_SET_CONTROL_PORT_REQ: u16 = 0x1404;
pub(crate) const ME_SET_CONTROL_PORT_CFM: u16 = 0x1405;
// The Linux driver may issue this request after a TX queue transition.  The
// firmware can return its confirmation asynchronously even when this Rust
// owner did not submit the optional traffic indication request, so it must not
// be mistaken for the confirmation of the active control mailbox.
pub(crate) const ME_TRAFFIC_IND_CFM: u16 = 0x140b;
pub(crate) const SM_CONNECT_REQ: u16 = 0x1800;
pub(crate) const SM_CONNECT_CFM: u16 = 0x1801;
pub(crate) const SM_CONNECT_IND: u16 = 0x1802;
pub(crate) const SM_DISCONNECT_REQ: u16 = 0x1803;
pub(crate) const SM_DISCONNECT_CFM: u16 = 0x1804;
pub(crate) const SM_DISCONNECT_IND: u16 = 0x1805;
// SCANU is the firmware's full-MAC scan task (task id 4, base 0x1000).
// Result frames are unsolicited and can arrive while a station request is
// being staged, so they must not be mistaken for a mailbox confirmation.
pub(crate) const SCANU_RESULT_IND: u16 = 0x1004;

pub(crate) const RSN_IE_CCMP_PSK: [u8; 22] = [
    0x30, 20, 1, 0, 0x00, 0x0f, 0xac, 4, 1, 0, 0x00, 0x0f, 0xac, 4, 1, 0, 0x00, 0x0f, 0xac, 2, 0, 0,
];

// `struct sm_connect_req` from the Linux AIC8800 driver is sent with the
// compiler's native C alignment.  In particular, `mac_addr` starts after the
// 33-byte SSID field, and `mac_chan_def` is six bytes (including its trailing
// two-byte alignment).  Keep the offsets in one place so the payload builder
// cannot silently drift when fields are added elsewhere.
const SM_CONNECT_PAYLOAD_LEN: usize = 320;
const SM_CONNECT_BSSID_OFFSET: usize = 34;
const SM_CONNECT_CHANNEL_OFFSET: usize = 40;
const SM_CONNECT_FLAGS_OFFSET: usize = 48;
const SM_CONNECT_CONTROL_PORT_OFFSET: usize = 52;
const SM_CONNECT_IE_LEN_OFFSET: usize = 54;
const SM_CONNECT_VIF_OFFSET: usize = 61;
const SM_CONNECT_IE_OFFSET: usize = 64;

pub(crate) struct ConnectIndication {
    pub(crate) bssid: [u8; 6],
    pub(crate) interface_index: u8,
    pub(crate) station_index: u8,
}

pub(crate) struct DisconnectIndication {
    pub(crate) reason_code: u16,
    pub(crate) interface_index: u8,
}

pub(crate) const fn is_indication_message(message_id: u16) -> bool {
    matches!(
        message_id,
        MM_PRIMARY_TBTT_IND
            | MM_SECONDARY_TBTT_IND
            | MM_CONNECTION_LOSS_IND
            | MM_CHANNEL_SWITCH_IND
            | MM_CHANNEL_PRE_SWITCH_IND
            | MM_REMAIN_ON_CHANNEL_EXP_IND
            | MM_PS_CHANGE_IND
            | MM_TRAFFIC_REQ_IND
            | MM_P2P_VIF_PS_CHANGE_IND
            | MM_CSA_COUNTER_IND
            | MM_CHANNEL_SURVEY_IND
            | MM_P2P_NOA_UPD_IND
            | MM_RSSI_STATUS_IND
            | MM_CSA_FINISH_IND
            | MM_CSA_TRAFFIC_IND
            | MM_PKTLOSS_IND
            | MM_APM_STALOSS_IND
            | MM_RADAR_DETECT_IND
            | ME_TRAFFIC_IND_CFM
            | SCANU_RESULT_IND
            | SM_CONNECT_IND
            | SM_DISCONNECT_IND
    )
}

pub(crate) fn require_empty(_message_id: u16, payload: &[u8]) -> Result<(), AicError> {
    if payload.is_empty() {
        Ok(())
    } else {
        Err(AicError::MalformedResponse)
    }
}

pub(crate) fn require_status_ok(message_id: u16, payload: &[u8]) -> Result<(), AicError> {
    let status = *payload.first().ok_or(AicError::MalformedResponse)?;
    if status == 0 {
        Ok(())
    } else {
        Err(AicError::FirmwareRejected {
            message_id,
            status: u16::from(status),
        })
    }
}

pub(crate) fn parse_mac(payload: &[u8]) -> Result<[u8; 6], AicError> {
    payload.try_into().map_err(|_| AicError::MalformedResponse)
}

pub(crate) fn parse_add_interface(payload: &[u8]) -> Result<u8, AicError> {
    if payload.len() != 2 {
        return Err(AicError::MalformedResponse);
    }
    require_status_ok(MM_ADD_IF_CFM, payload)?;
    (payload[1] != u8::MAX)
        .then_some(payload[1])
        .ok_or(AicError::MalformedResponse)
}

pub(crate) fn parse_ap_start(payload: &[u8], interface_index: u8) -> Result<(), AicError> {
    if payload.len() != 4 {
        return Err(AicError::MalformedResponse);
    }
    require_status_ok(APM_START_CFM, payload)?;
    if payload[1] != interface_index || payload[2] == u8::MAX || payload[3] == u8::MAX {
        return Err(AicError::MalformedResponse);
    }
    Ok(())
}

pub(crate) fn parse_connect_indication(payload: &[u8]) -> Result<ConnectIndication, AicError> {
    if payload.len() < 11 {
        return Err(AicError::MalformedResponse);
    }
    let status = u16::from_le_bytes([payload[0], payload[1]]);
    if status != 0 {
        return Err(AicError::FirmwareRejected {
            message_id: SM_CONNECT_IND,
            status,
        });
    }
    Ok(ConnectIndication {
        bssid: payload[2..8]
            .try_into()
            .map_err(|_| AicError::MalformedResponse)?,
        interface_index: payload[9],
        station_index: payload[10],
    })
}

pub(crate) fn parse_disconnect_indication(
    payload: &[u8],
) -> Result<DisconnectIndication, AicError> {
    if !matches!(payload.len(), 5 | 6) || payload.get(5).is_some_and(|padding| *padding != 0) {
        return Err(AicError::MalformedResponse);
    }
    let interface_index = payload[2];
    if interface_index == u8::MAX {
        return Err(AicError::MalformedResponse);
    }
    Ok(DisconnectIndication {
        reason_code: u16::from_le_bytes([payload[0], payload[1]]),
        interface_index,
    })
}

pub(crate) const fn stack_start_payload(vendor: u8) -> [u8; 4] {
    [1, 0, vendor, 0]
}

pub(crate) fn tx_power_level_payload() -> [u8; 95] {
    let mut payload = [0; 95];
    let profiles: [&[u8]; 6] = [
        &[20, 20, 20, 20, 20, 20, 20, 20, 18, 18, 16, 16],
        &[20, 20, 20, 20, 18, 18, 16, 16, 16, 16],
        &[20, 20, 20, 20, 18, 18, 16, 16, 16, 16, 15, 15],
        &[0x80, 0x80, 0x80, 0x80, 20, 20, 20, 20, 18, 18, 16, 16],
        &[20, 20, 20, 20, 18, 18, 16, 16, 16, 15],
        &[20, 20, 20, 20, 18, 18, 16, 16, 16, 15, 14, 14],
    ];
    payload[0] = 1;
    let mut offset = 1;
    for profile in profiles {
        payload[offset..offset + profile.len()].copy_from_slice(profile);
        offset += profile.len();
    }
    payload
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RfCalibrationBand {
    Ghz2Only,
    DualBand,
}

pub(crate) fn rf_calibration_payload(band: RfCalibrationBand) -> [u8; 24] {
    let mut payload = [0; 24];
    payload[0..4].copy_from_slice(&0x0000_0f8fu32.to_le_bytes());
    if band == RfCalibrationBand::DualBand {
        payload[4..8].copy_from_slice(&0x0000_0f0fu32.to_le_bytes());
    }
    payload[8..12].copy_from_slice(&0x0c34_c008u32.to_le_bytes());
    payload[16..20].copy_from_slice(&0x0026_4203u32.to_le_bytes());
    payload
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RfTableSelection {
    Receive  = 0,
    Transmit = 1,
}

pub(crate) fn rf_config_payload(
    selection: RfTableSelection,
    table_offset: u8,
    words: &[u32],
) -> Result<[u8; 260], AicError> {
    if words.len() > 64 {
        return Err(AicError::InvalidFirmwareAsset);
    }
    let mut payload = [0; 260];
    payload[0] = selection as u8;
    payload[1] = table_offset;
    payload[2] = 16;
    for (index, word) in words.iter().enumerate() {
        let offset = 4 + index * 4;
        payload[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(payload)
}

pub(crate) const fn get_mac_payload() -> [u8; 4] {
    1u32.to_le_bytes()
}

const ME_CONFIG_PAYLOAD_LEN: usize = 112;
const ME_CONFIG_HT_OFFSET: usize = 0;
const ME_CONFIG_VHT_OFFSET: usize = 32;
const ME_CONFIG_HE_OFFSET: usize = 44;
const ME_CONFIG_TX_LIFETIME_OFFSET: usize = 100;
const ME_CONFIG_PHY_BW_OFFSET: usize = 102;
const ME_CONFIG_HT_SUPPORTED_OFFSET: usize = 103;
const ME_CONFIG_VHT_SUPPORTED_OFFSET: usize = 104;
const ME_CONFIG_HE_SUPPORTED_OFFSET: usize = 105;
const ME_CONFIG_HE_UL_ON_OFFSET: usize = 106;
const ME_CONFIG_PS_ON_OFFSET: usize = 107;
const ME_CONFIG_ANT_DIV_ON_OFFSET: usize = 108;
const ME_CONFIG_DPSM_OFFSET: usize = 109;

const HT_CAPABILITY_INFO_OFFSET: usize = ME_CONFIG_HT_OFFSET;
const HT_AMPDU_PARAM_OFFSET: usize = ME_CONFIG_HT_OFFSET + 2;
const HT_MCS_OFFSET: usize = ME_CONFIG_HT_OFFSET + 3;
const HT_MCS_RX_MASK_LEN: usize = 10;
const HT_MCS_RX_HIGHEST_OFFSET: usize = HT_MCS_OFFSET + HT_MCS_RX_MASK_LEN;
const HT_MCS_TX_PARAMS_OFFSET: usize = HT_MCS_RX_HIGHEST_OFFSET + 2;
const HT_MCS_RESERVED_OFFSET: usize = HT_MCS_TX_PARAMS_OFFSET + 1;
const HT_CAP_LDPC: u16 = 0x0001;
const HT_CAP_WIDTH_20_40: u16 = 0x0002;
const HT_CAP_SGI_20: u16 = 0x0020;
const HT_CAP_SGI_40: u16 = 0x0040;
const HT_MCS_TX_DEFINED: u8 = 0x01;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeConfigProfile {
    Conservative,
    D80Ht40Sgi,
}

impl MeConfigProfile {
    pub(crate) const fn for_chip(chip: ChipVariant) -> Option<Self> {
        match chip {
            ChipVariant::Aic8800DC => Some(Self::Conservative),
            ChipVariant::Aic8800D80 => Some(Self::D80Ht40Sgi),
            ChipVariant::Aic8801
            | ChipVariant::Aic8800DW
            | ChipVariant::Aic8800D80X2
            | ChipVariant::Unknown => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Conservative => "conservative",
            Self::D80Ht40Sgi => "d80-ht40-sgi",
        }
    }

    const fn phy_bw_max(self) -> u8 {
        match self {
            Self::Conservative => 2, // PHY_CHNL_BW_80
            Self::D80Ht40Sgi => 1,   // PHY_CHNL_BW_40
        }
    }

    const fn ht_capabilities(self) -> HtCapabilities {
        match self {
            Self::Conservative => HtCapabilities::CONSERVATIVE,
            Self::D80Ht40Sgi => HtCapabilities::D80_HT40_SGI,
        }
    }
}

#[derive(Clone, Copy)]
struct AmpduParameters {
    max_length_factor: u8,
    minimum_spacing: u8,
}

impl AmpduParameters {
    const VENDOR_DEFAULT: Self = Self {
        max_length_factor: 3,
        minimum_spacing: 7,
    };

    const fn encode(self) -> u8 {
        self.max_length_factor | (self.minimum_spacing << 2)
    }
}

#[derive(Clone, Copy)]
struct HtCapabilities {
    capability_info: u16,
    ampdu: AmpduParameters,
    rx_mask: [u8; HT_MCS_RX_MASK_LEN],
    rx_highest: u16,
    tx_params: u8,
}

impl HtCapabilities {
    const CONSERVATIVE: Self = Self {
        capability_info: HT_CAP_LDPC,
        ampdu: AmpduParameters::VENDOR_DEFAULT,
        rx_mask: [0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        rx_highest: 65,
        tx_params: HT_MCS_TX_DEFINED,
    };

    const D80_HT40_SGI: Self = Self {
        capability_info: HT_CAP_LDPC | HT_CAP_WIDTH_20_40 | HT_CAP_SGI_20 | HT_CAP_SGI_40,
        ampdu: AmpduParameters::VENDOR_DEFAULT,
        rx_mask: [0xff, 0, 0, 0, 1, 0, 0, 0, 0, 0],
        rx_highest: 150,
        tx_params: HT_MCS_TX_DEFINED,
    };

    fn encode_into(self, payload: &mut [u8; ME_CONFIG_PAYLOAD_LEN]) {
        payload[HT_CAPABILITY_INFO_OFFSET..HT_CAPABILITY_INFO_OFFSET + 2]
            .copy_from_slice(&self.capability_info.to_le_bytes());
        payload[HT_AMPDU_PARAM_OFFSET] = self.ampdu.encode();
        payload[HT_MCS_OFFSET..HT_MCS_OFFSET + self.rx_mask.len()].copy_from_slice(&self.rx_mask);
        payload[HT_MCS_RX_HIGHEST_OFFSET..HT_MCS_RX_HIGHEST_OFFSET + 2]
            .copy_from_slice(&self.rx_highest.to_le_bytes());
        payload[HT_MCS_TX_PARAMS_OFFSET] = self.tx_params;
        payload[HT_MCS_RESERVED_OFFSET..HT_MCS_OFFSET + 16].fill(0);
    }
}

pub(crate) fn me_config_payload(profile: MeConfigProfile) -> [u8; ME_CONFIG_PAYLOAD_LEN] {
    let mut payload = [0; ME_CONFIG_PAYLOAD_LEN];
    profile.ht_capabilities().encode_into(&mut payload);

    // These capability structures are naturally aligned in the vendor C ABI:
    // HT occupies 32 bytes, VHT 12 bytes, and HE 56 bytes before tx_lft.
    payload[ME_CONFIG_VHT_OFFSET..ME_CONFIG_HE_OFFSET].fill(0);
    payload[ME_CONFIG_HE_OFFSET..ME_CONFIG_TX_LIFETIME_OFFSET].fill(0);
    payload[ME_CONFIG_TX_LIFETIME_OFFSET..ME_CONFIG_TX_LIFETIME_OFFSET + 2]
        .copy_from_slice(&1000u16.to_le_bytes());
    payload[ME_CONFIG_PHY_BW_OFFSET] = profile.phy_bw_max();
    payload[ME_CONFIG_HT_SUPPORTED_OFFSET] = 1;
    payload[ME_CONFIG_VHT_SUPPORTED_OFFSET] = 0;
    payload[ME_CONFIG_HE_SUPPORTED_OFFSET] = 0;
    payload[ME_CONFIG_HE_UL_ON_OFFSET] = 0;
    payload[ME_CONFIG_PS_ON_OFFSET] = 1;
    payload[ME_CONFIG_ANT_DIV_ON_OFFSET] = 0;
    payload[ME_CONFIG_DPSM_OFFSET] = 0;
    payload
}

pub(crate) fn channel_config_payload() -> [u8; 254] {
    let mut payload = [0; 254];
    const CHANNELS: [u16; 14] = [
        2412, 2417, 2422, 2427, 2432, 2437, 2442, 2447, 2452, 2457, 2462, 2467, 2472, 2484,
    ];
    for (index, frequency) in CHANNELS.into_iter().enumerate() {
        let offset = index * 6;
        payload[offset..offset + 2].copy_from_slice(&frequency.to_le_bytes());
        payload[offset + 4] = 30;
    }
    payload[252] = CHANNELS.len() as u8;
    payload
}

pub(crate) fn add_interface_payload(mac: [u8; 6], role: u8) -> [u8; 10] {
    let mut payload = [0; 10];
    payload[0] = role;
    payload[2..8].copy_from_slice(&mac);
    payload
}

pub(crate) fn connect_payload(ssid: &[u8], secured: bool, interface_index: u8) -> Vec<u8> {
    let mut payload = vec![0; SM_CONNECT_PAYLOAD_LEN];
    payload[0] = ssid.len() as u8;
    payload[1..1 + ssid.len()].copy_from_slice(ssid);
    payload[SM_CONNECT_BSSID_OFFSET..SM_CONNECT_BSSID_OFFSET + 6].fill(0xff);
    payload[SM_CONNECT_CHANNEL_OFFSET..SM_CONNECT_CHANNEL_OFFSET + 2]
        .copy_from_slice(&0xffffu16.to_le_bytes());
    if secured {
        // CONTROL_PORT_HOST | CONTROL_PORT_NO_ENC | WPA_WPA2.
        payload[SM_CONNECT_FLAGS_OFFSET..SM_CONNECT_FLAGS_OFFSET + 4]
            .copy_from_slice(&0x0000_000bu32.to_le_bytes());
        payload[SM_CONNECT_IE_OFFSET..SM_CONNECT_IE_OFFSET + RSN_IE_CCMP_PSK.len()]
            .copy_from_slice(&RSN_IE_CCMP_PSK);
        payload[SM_CONNECT_IE_LEN_OFFSET..SM_CONNECT_IE_LEN_OFFSET + 2]
            .copy_from_slice(&(RSN_IE_CCMP_PSK.len() as u16).to_le_bytes());
    }
    payload[SM_CONNECT_CONTROL_PORT_OFFSET..SM_CONNECT_CONTROL_PORT_OFFSET + 2]
        .copy_from_slice(&0x888eu16.to_be_bytes());
    payload[SM_CONNECT_VIF_OFFSET] = interface_index;
    payload
}

pub(crate) const fn control_port_payload(station_index: u8, open: bool) -> [u8; 2] {
    [station_index, open as u8]
}

pub(crate) fn disconnect_payload(interface_index: u8) -> [u8; 4] {
    [3, 0, interface_index, 0]
}

pub(crate) fn key_add_payload(
    interface_index: u8,
    station_index: u8,
    pairwise: bool,
    key_index: u8,
    key: &[u8],
) -> Result<[u8; 44], AicError> {
    if key.len() != 16 {
        return Err(AicError::WpaKeyData);
    }
    let mut payload = [0; 44];
    payload[0] = key_index;
    payload[1] = station_index;
    payload[4] = key.len() as u8;
    payload[8..8 + key.len()].copy_from_slice(key);
    payload[40] = 2; // MAC_CIPHER_CCMP
    payload[41] = interface_index;
    payload[43] = pairwise as u8;
    Ok(payload)
}

pub(crate) fn parse_key_add_confirmation(payload: &[u8]) -> Result<u8, AicError> {
    // Linux's `struct mm_key_add_cfm` is `{ u8 status; u8 hw_key_idx; }`.
    // The firmware emits exactly these two bytes; accepting a padded variant
    // would hide a transport/layout mismatch.
    if payload.len() != 2 {
        return Err(AicError::MalformedResponse);
    }
    require_status_ok(MM_KEY_ADD_CFM, payload)?;
    (payload[1] != u8::MAX)
        .then_some(payload[1])
        .ok_or(AicError::MalformedResponse)
}

// ============================================================
// Station information telemetry (board measurement)
// ============================================================

/// Firmware request that reports the rate currently used for a peer.
pub(crate) const MM_GET_STA_INFO_REQ: u16 = 0x0075;
pub(crate) const MM_GET_STA_INFO_CFM: u16 = 0x0076;

/// The vendor sends `struct mm_get_sta_info_req` (one byte) only for chips at
/// or above D80X2; every other variant, including both chips this driver
/// supports, sends `struct mm_get_sta_info_compat_req`, which is the station
/// index followed by the ASCII tag "sta".
const STA_INFO_COMPAT_TAG: [u8; 3] = *b"sta";
const STA_INFO_PAYLOAD_LEN: usize = 1 + STA_INFO_COMPAT_TAG.len();

/// `struct mm_get_sta_info_cfm` is a plain 32 byte structure whose fields are
/// already naturally aligned, so it has no padding to skip.
const STA_INFO_CONFIRMATION_LEN: usize = 32;

// `union rwnx_rate_ctrl_info` packs the transmit rate into one word; the
// fields are declared least significant first.  Only the transmit half is
// decoded here: the protected-half fields describe the protection frame.
const RATE_INFO_MCS_SHIFT: u32 = 0;
const RATE_INFO_MCS_MASK: u32 = 0x7f;
const RATE_INFO_WIDTH_SHIFT: u32 = 7;
const RATE_INFO_WIDTH_MASK: u32 = 0x03;
const RATE_INFO_SHORT_GUARD_SHIFT: u32 = 9;
const RATE_INFO_FORMAT_SHIFT: u32 = 11;
const RATE_INFO_FORMAT_MASK: u32 = 0x07;
const RATE_INFO_RETRY_SHIFT: u32 = 29;
const RATE_INFO_RETRY_MASK: u32 = 0x07;

const fn rate_field(word: u32, shift: u32, mask: u32) -> u32 {
    (word >> shift) & mask
}

/// Channel width the firmware reports for the current transmit rate
/// (`enum mac_chan_bandwidth`, which the two width bits can hold entirely).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChannelWidth {
    Mhz20,
    Mhz40,
    Mhz80,
    Mhz160,
}

impl ChannelWidth {
    const fn from_firmware(value: u32) -> Self {
        match value {
            1 => Self::Mhz40,
            2 => Self::Mhz80,
            3 => Self::Mhz160,
            _ => Self::Mhz20,
        }
    }

    pub(crate) const fn mhz(self) -> u16 {
        match self {
            Self::Mhz20 => 20,
            Self::Mhz40 => 40,
            Self::Mhz80 => 80,
            Self::Mhz160 => 160,
        }
    }
}

/// Modulation format the firmware reports (`FORMATMOD_*` values).  The three
/// format bits hold every value the vendor defines but `HE_TB`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TxFormat {
    NonHt,
    NonHtDuplicateOfdm,
    HtMixed,
    HtGreenfield,
    Vht,
    HeSu,
    HeMu,
    HeEr,
}

impl TxFormat {
    const fn from_firmware(value: u32) -> Self {
        match value {
            1 => Self::NonHtDuplicateOfdm,
            2 => Self::HtMixed,
            3 => Self::HtGreenfield,
            4 => Self::Vht,
            5 => Self::HeSu,
            6 => Self::HeMu,
            7 => Self::HeEr,
            _ => Self::NonHt,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::NonHt => "non-ht",
            Self::NonHtDuplicateOfdm => "non-ht-dup-ofdm",
            Self::HtMixed => "ht-mf",
            Self::HtGreenfield => "ht-gf",
            Self::Vht => "vht",
            Self::HeSu => "he-su",
            Self::HeMu => "he-mu",
            Self::HeEr => "he-er",
        }
    }
}

/// The part of one sample that answers "which rate is the firmware sending
/// at": a change here is what a board round reads the log for, while the
/// counters move on every sample by construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TxRateDescriptor {
    pub(crate) width: ChannelWidth,
    pub(crate) format: TxFormat,
    /// Modulation index inside `format`; a legacy rate index when the format
    /// is not HT, VHT or HE.
    pub(crate) mcs: u8,
    /// Spatial streams the index encodes; one for a legacy rate.
    pub(crate) streams: u8,
    pub(crate) short_guard_interval: bool,
    pub(crate) rssi: i8,
}

impl TxRateDescriptor {
    /// The index field is shared with the stream count, and how the two are
    /// packed changed with the format: HT keeps the index in the low three
    /// bits with the stream count above it, VHT and HE use four bits for the
    /// index, and a legacy rate is the index on its own.
    const fn mcs_and_streams(index: u8, format: TxFormat) -> (u8, u8) {
        match format {
            TxFormat::NonHt | TxFormat::NonHtDuplicateOfdm => (index, 1),
            TxFormat::HtMixed | TxFormat::HtGreenfield => (index & 0x7, ((index >> 3) & 0x7) + 1),
            TxFormat::Vht | TxFormat::HeSu | TxFormat::HeMu | TxFormat::HeEr => {
                (index & 0xf, ((index >> 4) & 0x7) + 1)
            }
        }
    }

    const fn from_rate_info(word: u32) -> Self {
        let format = TxFormat::from_firmware(rate_field(
            word,
            RATE_INFO_FORMAT_SHIFT,
            RATE_INFO_FORMAT_MASK,
        ));
        let (mcs, streams) = Self::mcs_and_streams(
            rate_field(word, RATE_INFO_MCS_SHIFT, RATE_INFO_MCS_MASK) as u8,
            format,
        );
        Self {
            width: ChannelWidth::from_firmware(rate_field(
                word,
                RATE_INFO_WIDTH_SHIFT,
                RATE_INFO_WIDTH_MASK,
            )),
            format,
            mcs,
            streams,
            short_guard_interval: rate_field(word, RATE_INFO_SHORT_GUARD_SHIFT, 1) != 0,
            rssi: 0,
        }
    }
}

/// One decoded `MM_GET_STA_INFO_CFM`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StationInfo {
    pub(crate) rate: TxRateDescriptor,
    pub(crate) retries: u8,
    pub(crate) tx_failed: u32,
    pub(crate) acknowledge_failed: u32,
    pub(crate) acknowledge_succeeded: u32,
}

impl core::fmt::Display for StationInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "width={}MHz format={} mcs={} nss={} sgi={} retries={} rssi={}dBm txfailed={} \
             ackok={} ackfail={}",
            self.rate.width.mhz(),
            self.rate.format.name(),
            self.rate.mcs,
            self.rate.streams,
            u8::from(self.rate.short_guard_interval),
            self.retries,
            self.rate.rssi,
            self.tx_failed,
            self.acknowledge_succeeded,
            self.acknowledge_failed,
        )
    }
}

impl StationInfo {
    /// The rate fields alone, for deciding whether a sample is worth printing.
    pub(crate) const fn descriptor(&self) -> TxRateDescriptor {
        self.rate
    }
}

pub(crate) const fn sta_info_payload(station_index: u8) -> [u8; STA_INFO_PAYLOAD_LEN] {
    [
        station_index,
        STA_INFO_COMPAT_TAG[0],
        STA_INFO_COMPAT_TAG[1],
        STA_INFO_COMPAT_TAG[2],
    ]
}

pub(crate) fn parse_sta_info(payload: &[u8]) -> Result<StationInfo, AicError> {
    if payload.len() != STA_INFO_CONFIRMATION_LEN {
        return Err(AicError::MalformedResponse);
    }
    let word = |offset: usize| {
        u32::from_le_bytes(
            payload[offset..offset + 4]
                .try_into()
                .expect("the confirmation length was checked above"),
        )
    };
    let rate_info = word(0);
    Ok(StationInfo {
        rate: TxRateDescriptor {
            rssi: payload[8] as i8,
            ..TxRateDescriptor::from_rate_info(rate_info)
        },
        retries: rate_field(rate_info, RATE_INFO_RETRY_SHIFT, RATE_INFO_RETRY_MASK) as u8,
        tx_failed: word(4),
        acknowledge_failed: word(20),
        acknowledge_succeeded: word(24),
    })
}

pub(crate) const fn filter_payload() -> [u8; 4] {
    0x1502_868cu32.to_le_bytes()
}

pub(crate) const fn start_payload() -> [u8; 72] {
    let mut payload = [0; 72];
    let timeout = 300u32.to_le_bytes();
    let clock_accuracy = 20u16.to_le_bytes();
    payload[64] = timeout[0];
    payload[65] = timeout[1];
    payload[66] = timeout[2];
    payload[67] = timeout[3];
    payload[68] = clock_accuracy[0];
    payload[69] = clock_accuracy[1];
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_confirmation_status_is_not_an_association_result() {
        assert_eq!(require_status_ok(SM_CONNECT_CFM, &[0]), Ok(()));
        assert_eq!(
            require_status_ok(SM_CONNECT_CFM, &[7]),
            Err(AicError::FirmwareRejected {
                message_id: SM_CONNECT_CFM,
                status: 7,
            })
        );
    }

    #[test]
    fn add_interface_rejects_invalid_firmware_index() {
        assert_eq!(
            parse_add_interface(&[0, u8::MAX]),
            Err(AicError::MalformedResponse)
        );
    }

    #[test]
    fn ap_start_confirmation_requires_the_requested_vif_and_complete_firmware_layout() {
        assert_eq!(parse_ap_start(&[0, 1, 2, 3], 1), Ok(()));
        for payload in [
            &[0][..],
            &[0, 1, 2, 3, 0],
            &[0, 2, 2, 3],
            &[0, 1, 255, 3],
            &[0, 1, 2, 255],
        ] {
            assert_eq!(parse_ap_start(payload, 1), Err(AicError::MalformedResponse));
        }
        assert_eq!(
            parse_ap_start(&[5, 1, 2, 3], 1),
            Err(AicError::FirmwareRejected {
                message_id: APM_START_CFM,
                status: 5
            })
        );
    }

    #[test]
    fn d80_rf_calibration_matches_the_vendor_request() {
        let payload = rf_calibration_payload(RfCalibrationBand::DualBand);

        assert_eq!(payload.len(), 24);
        assert_eq!(&payload[0..4], &0x0000_0f8fu32.to_le_bytes());
        assert_eq!(&payload[4..8], &0x0000_0f0fu32.to_le_bytes());
        assert_eq!(&payload[8..12], &0x0c34_c008u32.to_le_bytes());
        assert_eq!(&payload[12..16], &0u32.to_le_bytes());
        assert_eq!(&payload[16..20], &0x0026_4203u32.to_le_bytes());
        assert_eq!(&payload[20..24], &[0; 4]);
    }

    #[test]
    fn dc_rf_calibration_matches_the_2ghz_only_vendor_request() {
        let payload = rf_calibration_payload(RfCalibrationBand::Ghz2Only);

        assert_eq!(&payload[0..4], &0x0000_0f8fu32.to_le_bytes());
        assert_eq!(&payload[4..8], &0u32.to_le_bytes());
    }

    #[test]
    fn dc_rf_config_uses_the_vendor_c_layout() {
        let payload = rf_config_payload(RfTableSelection::Transmit, 16, &[0x1122_3344]).unwrap();

        assert_eq!(&payload[..4], &[1, 16, 16, 0]);
        assert_eq!(&payload[4..8], &0x1122_3344u32.to_le_bytes());
        assert_eq!(&payload[8..], &[0; 252]);
    }

    #[test]
    fn secured_connect_uses_the_vendor_sm_connect_layout() {
        let payload = connect_payload(b"network", true, 6);

        assert_eq!(payload.len(), 320);
        assert_eq!(payload[33], 0);
        assert_eq!(&payload[34..40], &[0xff; 6]);
        assert_eq!(&payload[40..42], &0xffffu16.to_le_bytes());
        assert_eq!(&payload[48..52], &0x0000_000bu32.to_le_bytes());
        assert_eq!(&payload[52..54], &0x888eu16.to_be_bytes());
        assert_eq!(
            &payload[54..56],
            &(RSN_IE_CCMP_PSK.len() as u16).to_le_bytes()
        );
        assert_eq!(&payload[56..61], &[0; 5]);
        assert_eq!(payload[61], 6);
        assert_eq!(&payload[62..64], &[0; 2]);
        assert_eq!(&payload[64..64 + RSN_IE_CCMP_PSK.len()], &RSN_IE_CCMP_PSK);
    }

    #[test]
    fn mac_start_uses_the_vendor_runtime_defaults() {
        let payload = start_payload();

        assert_eq!(payload.len(), 72);
        assert_eq!(&payload[..64], &[0; 64]);
        assert_eq!(&payload[64..68], &300u32.to_le_bytes());
        assert_eq!(&payload[68..70], &20u16.to_le_bytes());
        assert_eq!(&payload[70..72], &[0; 2]);
    }

    #[test]
    fn key_add_uses_the_vendor_mm_key_add_layout() {
        let key = [0x5a; 16];
        let payload = key_add_payload(2, 7, true, 0, &key).unwrap();

        assert_eq!(payload.len(), 44);
        assert_eq!(payload[0], 0);
        assert_eq!(payload[1], 7);
        assert_eq!(payload[4], key.len() as u8);
        assert_eq!(&payload[8..24], &key);
        assert_eq!(payload[40], 2);
        assert_eq!(payload[41], 2);
        assert_eq!(payload[42], 0);
        assert_eq!(payload[43], 1);
    }

    /// One `struct mm_get_sta_info_cfm` with the fields a board round reads.
    fn sta_info_confirmation(
        rate_info: u32,
        rssi: i8,
        tx_failed: u32,
        acknowledge_failed: u32,
        acknowledge_succeeded: u32,
    ) -> [u8; STA_INFO_CONFIRMATION_LEN] {
        let mut payload = [0; STA_INFO_CONFIRMATION_LEN];
        payload[0..4].copy_from_slice(&rate_info.to_le_bytes());
        payload[4..8].copy_from_slice(&tx_failed.to_le_bytes());
        payload[8] = rssi as u8;
        payload[20..24].copy_from_slice(&acknowledge_failed.to_le_bytes());
        payload[24..28].copy_from_slice(&acknowledge_succeeded.to_le_bytes());
        payload
    }

    /// One `union rwnx_rate_ctrl_info` word, fields laid out the vendor way.
    fn rate_info(
        width: u32,
        format: u32,
        index: u8,
        short_guard_interval: bool,
        retries: u8,
    ) -> u32 {
        (width << 7)
            | (format << 11)
            | (u32::from(short_guard_interval) << 9)
            | u32::from(index)
            | (u32::from(retries) << 29)
    }

    #[test]
    fn sta_info_request_carries_the_station_index_and_the_vendor_compat_tag() {
        // The vendor sends the four byte compatibility form for every chip
        // below D80X2, which is both chips this driver supports.
        assert_eq!(sta_info_payload(3), [3, b's', b't', b'a']);
    }

    #[test]
    fn station_info_decodes_the_rate_of_each_format_and_its_counters() {
        // HT packs the stream count above a three bit index: two streams at
        // MCS 7 is `(2 - 1) << 3 | 7`.
        let ht = parse_sta_info(&sta_info_confirmation(
            rate_info(1, 2, 0b1111, true, 3),
            -52,
            5,
            7,
            4242,
        ))
        .unwrap();
        assert_eq!(ht.rate.width, ChannelWidth::Mhz40);
        assert_eq!(ht.rate.format, TxFormat::HtMixed);
        assert_eq!(ht.rate.mcs, 7);
        assert_eq!(ht.rate.streams, 2);
        assert!(ht.rate.short_guard_interval);
        assert_eq!(ht.rate.rssi, -52);
        assert_eq!(ht.retries, 3);
        assert_eq!(ht.tx_failed, 5);
        assert_eq!(ht.acknowledge_failed, 7);
        assert_eq!(ht.acknowledge_succeeded, 4242);

        // VHT and HE use four bits for the index: two streams at MCS 9 is
        // `(2 - 1) << 4 | 9`.
        let vht = parse_sta_info(&sta_info_confirmation(
            rate_info(2, 4, 0b1_1001, false, 0),
            -40,
            0,
            0,
            1,
        ))
        .unwrap();
        assert_eq!(vht.rate.width, ChannelWidth::Mhz80);
        assert_eq!(vht.rate.format, TxFormat::Vht);
        assert_eq!(vht.rate.mcs, 9);
        assert_eq!(vht.rate.streams, 2);
        assert!(!vht.rate.short_guard_interval);

        // A legacy rate carries the index alone and is always one stream.
        let legacy = parse_sta_info(&sta_info_confirmation(
            rate_info(0, 0, 3, false, 0),
            -30,
            0,
            0,
            0,
        ))
        .unwrap();
        assert_eq!(legacy.rate.format, TxFormat::NonHt);
        assert_eq!(legacy.rate.width, ChannelWidth::Mhz20);
        assert_eq!(legacy.rate.mcs, 3);
        assert_eq!(legacy.rate.streams, 1);
    }

    #[test]
    fn station_info_rejects_a_confirmation_of_the_wrong_length() {
        let payload = sta_info_confirmation(0, 0, 0, 0, 0);
        assert_eq!(
            parse_sta_info(&payload[..STA_INFO_CONFIRMATION_LEN - 1]),
            Err(AicError::MalformedResponse)
        );
        let mut padded = payload.to_vec();
        padded.push(0);
        assert_eq!(parse_sta_info(&padded), Err(AicError::MalformedResponse));
    }

    #[test]
    fn disconnect_message_ids_follow_the_vendor_sm_enum() {
        assert_eq!(SM_DISCONNECT_REQ, 0x1803);
        assert_eq!(SM_DISCONNECT_CFM, 0x1804);
        assert_eq!(SM_DISCONNECT_IND, 0x1805);
    }

    #[test]
    fn d80_tx_power_profile_matches_the_vendor_defaults() {
        let payload = tx_power_level_payload();

        assert_eq!(payload.len(), 95);
        assert_eq!(payload[0], 1);
        assert_eq!(
            &payload[1..13],
            &[20, 20, 20, 20, 20, 20, 20, 20, 18, 18, 16, 16]
        );
        assert_eq!(
            &payload[35..47],
            &[0x80, 0x80, 0x80, 0x80, 20, 20, 20, 20, 18, 18, 16, 16]
        );
        assert_eq!(&payload[69..], &[0; 26]);
    }

    #[test]
    fn me_config_profiles_encode_the_vendor_ht_and_scalar_fields() {
        let conservative = me_config_payload(MeConfigProfile::Conservative);
        let d80 = me_config_payload(MeConfigProfile::D80Ht40Sgi);
        let expected_ampdu = [3 | (7 << 2)];
        let mut p0a_reference = [0; 112];
        p0a_reference[0..2].copy_from_slice(&1u16.to_le_bytes());
        p0a_reference[2] = 31;
        p0a_reference[3] = 0xff;
        p0a_reference[13..15].copy_from_slice(&65u16.to_le_bytes());
        p0a_reference[15] = 1;
        p0a_reference[100..102].copy_from_slice(&1000u16.to_le_bytes());
        p0a_reference[102] = 2;
        p0a_reference[103] = 1;
        p0a_reference[107] = 1;
        assert_eq!(conservative, p0a_reference);

        let mut d80_reference = p0a_reference;
        d80_reference[0..2].copy_from_slice(&0x0063u16.to_le_bytes());
        d80_reference[7] = 1; // MCS32 in rx_mask[4].
        d80_reference[13..15].copy_from_slice(&150u16.to_le_bytes());
        d80_reference[102] = 1; // PHY_CHNL_BW_40.
        assert_eq!(d80, d80_reference);

        assert_eq!(conservative.len(), ME_CONFIG_PAYLOAD_LEN);
        assert_eq!(d80.len(), ME_CONFIG_PAYLOAD_LEN);
        assert_eq!(
            &conservative[HT_CAPABILITY_INFO_OFFSET..HT_CAPABILITY_INFO_OFFSET + 2],
            &HT_CAP_LDPC.to_le_bytes()
        );
        assert_eq!(
            &d80[HT_CAPABILITY_INFO_OFFSET..HT_CAPABILITY_INFO_OFFSET + 2],
            &(HT_CAP_LDPC | HT_CAP_WIDTH_20_40 | HT_CAP_SGI_20 | HT_CAP_SGI_40).to_le_bytes()
        );
        assert_eq!(
            conservative[HT_AMPDU_PARAM_OFFSET..HT_AMPDU_PARAM_OFFSET + 1],
            expected_ampdu
        );
        assert_eq!(
            d80[HT_AMPDU_PARAM_OFFSET..HT_AMPDU_PARAM_OFFSET + 1],
            expected_ampdu
        );
        assert_eq!(
            &conservative[HT_MCS_OFFSET..HT_MCS_OFFSET + HT_MCS_RX_MASK_LEN],
            &[0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            &d80[HT_MCS_OFFSET..HT_MCS_OFFSET + HT_MCS_RX_MASK_LEN],
            &[0xff, 0, 0, 0, 1, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            &conservative[HT_MCS_RX_HIGHEST_OFFSET..HT_MCS_RX_HIGHEST_OFFSET + 2],
            &65u16.to_le_bytes()
        );
        assert_eq!(
            &d80[HT_MCS_RX_HIGHEST_OFFSET..HT_MCS_RX_HIGHEST_OFFSET + 2],
            &150u16.to_le_bytes()
        );
        assert_eq!(conservative[HT_MCS_TX_PARAMS_OFFSET], HT_MCS_TX_DEFINED);
        assert_eq!(d80[HT_MCS_TX_PARAMS_OFFSET], HT_MCS_TX_DEFINED);
        assert_eq!(
            &d80[HT_MCS_RESERVED_OFFSET..ME_CONFIG_VHT_OFFSET],
            &[0; 32 - HT_MCS_RESERVED_OFFSET]
        );
        assert_eq!(&d80[ME_CONFIG_VHT_OFFSET..ME_CONFIG_HE_OFFSET], &[0; 12]);
        assert_eq!(
            &d80[ME_CONFIG_HE_OFFSET..ME_CONFIG_TX_LIFETIME_OFFSET],
            &[0; 56]
        );
        assert_eq!(
            &conservative[ME_CONFIG_TX_LIFETIME_OFFSET..ME_CONFIG_TX_LIFETIME_OFFSET + 2],
            &1000u16.to_le_bytes()
        );
        assert_eq!(
            &d80[ME_CONFIG_TX_LIFETIME_OFFSET..ME_CONFIG_TX_LIFETIME_OFFSET + 2],
            &1000u16.to_le_bytes()
        );
        assert_eq!(conservative[ME_CONFIG_PHY_BW_OFFSET], 2);
        assert_eq!(d80[ME_CONFIG_PHY_BW_OFFSET], 1);
        assert_eq!(d80[ME_CONFIG_HT_SUPPORTED_OFFSET], 1);
        assert_eq!(
            &d80[ME_CONFIG_VHT_SUPPORTED_OFFSET..ME_CONFIG_HE_UL_ON_OFFSET + 1],
            &[0; 3]
        );
        assert_eq!(d80[ME_CONFIG_PS_ON_OFFSET], 1);
        assert_eq!(d80[ME_CONFIG_ANT_DIV_ON_OFFSET], 0);
        assert_eq!(d80[ME_CONFIG_DPSM_OFFSET], 0);
        assert_eq!(&d80[110..ME_CONFIG_PAYLOAD_LEN], &[0; 2]);
    }

    #[test]
    fn me_config_profile_selection_is_limited_to_validated_chip_variants() {
        assert_eq!(
            MeConfigProfile::for_chip(ChipVariant::Aic8800DC),
            Some(MeConfigProfile::Conservative)
        );
        assert_eq!(
            MeConfigProfile::for_chip(ChipVariant::Aic8800D80),
            Some(MeConfigProfile::D80Ht40Sgi)
        );
        for chip in [
            ChipVariant::Aic8801,
            ChipVariant::Aic8800DW,
            ChipVariant::Aic8800D80X2,
            ChipVariant::Unknown,
        ] {
            assert_eq!(MeConfigProfile::for_chip(chip), None);
        }
    }

    #[test]
    fn channel_config_uses_six_byte_vendor_channel_entries() {
        let payload = channel_config_payload();

        assert_eq!(payload.len(), 254);
        assert_eq!(&payload[0..2], &2412u16.to_le_bytes());
        assert_eq!(&payload[6..8], &2417u16.to_le_bytes());
        assert_eq!(payload[4], 30);
        assert_eq!(payload[252], 14);
        assert_eq!(payload[253], 0);
    }

    #[test]
    fn disconnect_request_and_key_confirmation_use_exact_vendor_sizes() {
        let disconnect = disconnect_payload(6);
        assert_eq!(disconnect.len(), 4);
        assert_eq!(disconnect.as_slice(), &[3, 0, 6, 0]);
        assert_eq!(parse_key_add_confirmation(&[0, 1]), Ok(1));
        assert_eq!(
            parse_key_add_confirmation(&[0, 1, 0, 0]),
            Err(AicError::MalformedResponse)
        );
    }

    #[test]
    fn asynchronous_traffic_confirmation_is_not_a_control_mailbox_result() {
        assert!(is_indication_message(ME_TRAFFIC_IND_CFM));
        assert!(!is_indication_message(ME_SET_CONTROL_PORT_CFM));
    }
}
