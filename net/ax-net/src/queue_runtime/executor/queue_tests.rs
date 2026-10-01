use alloc::{boxed::Box, vec};
use core::{alloc::Layout, num::NonZeroUsize, ptr::NonNull, sync::atomic::AtomicBool};
use std::sync::Mutex;

use rd_net::{
    FixedNetControl, IRxQueue, ITxQueue, NetDevice, NetDeviceInfo, NetDeviceParts,
    NetHardIrqEndpoint, NetHardIrqHandler, NetHardIrqResult, NetIrqSourceId, NetOwnerStartup,
    NetOwnerStartupProgress, NetPollGroupId, NetPollGroupParts, NetPollIrqControl, NetQueueId,
    NetQueuePairParts, QueueConfig, SubmitError, TxNotify,
    dma_api::{
        DeviceDma, DmaAllocHandle, DmaCoherency, DmaConstraints, DmaDeviceInfo, DmaDirection,
        DmaDomainId, DmaError, DmaMapHandle, DmaOp,
    },
};

use super::*;
use crate::queue_runtime::{
    NetQueueIdentity, install_queue_poll_observer, publish_queue_poll_gate, spsc_ring,
    tests::TEST_DMA,
};

type Trace = Arc<Mutex<Vec<&'static str>>>;

fn test_identity() -> NetQueueIdentity {
    NetQueueIdentity {
        discovery_order: 0,
        group_id: NetPollGroupId::new(0),
        owner_cpu: 0,
    }
}

struct FailingDma(AtomicBool);

impl DmaOp for FailingDma {
    fn page_size(&self) -> usize {
        TEST_DMA.page_size()
    }

    unsafe fn alloc_contiguous(
        &self,
        constraints: DmaConstraints,
        layout: Layout,
    ) -> Option<DmaAllocHandle> {
        if self.0.load(Ordering::Relaxed) {
            return None;
        }
        // SAFETY: forward the caller's allocation contract to the host allocator.
        unsafe { TEST_DMA.alloc_contiguous(constraints, layout) }
    }

    unsafe fn dealloc_contiguous(&self, handle: DmaAllocHandle) {
        // SAFETY: every successful allocation came from TEST_DMA.
        unsafe { TEST_DMA.dealloc_contiguous(handle) }
    }

    unsafe fn alloc_coherent(
        &self,
        constraints: DmaConstraints,
        layout: Layout,
    ) -> Option<DmaAllocHandle> {
        if self.0.load(Ordering::Relaxed) {
            return None;
        }
        // SAFETY: forward the caller's allocation contract to the host allocator.
        unsafe { TEST_DMA.alloc_coherent(constraints, layout) }
    }

    unsafe fn dealloc_coherent(&self, handle: DmaAllocHandle) -> Result<(), DmaError> {
        // SAFETY: every successful allocation came from TEST_DMA.
        unsafe { TEST_DMA.dealloc_coherent(handle) }
    }

    unsafe fn map_streaming(
        &self,
        constraints: DmaConstraints,
        addr: NonNull<u8>,
        size: NonZeroUsize,
        direction: DmaDirection,
    ) -> Result<DmaMapHandle, DmaError> {
        // SAFETY: preserve the caller's live buffer, size and direction.
        unsafe { TEST_DMA.map_streaming(constraints, addr, size, direction) }
    }

    unsafe fn unmap_streaming(&self, handle: DmaMapHandle) {
        // SAFETY: this handle was created by the delegated map_streaming.
        unsafe { TEST_DMA.unmap_streaming(handle) }
    }
}

#[test]
fn rx_allocation_failure_recovers_without_disabling_tx() {
    static DMA: FailingDma = FailingDma(AtomicBool::new(false));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let dma = DeviceDma::new(
        DmaDeviceInfo::new(
            DmaDomainId::Direct,
            DmaCoherency::Coherent,
            DmaConstraints::new(u64::MAX),
        ),
        &DMA,
    );
    let mut device = rd_net::prepare_device(
        Box::new(TestDevice(Arc::clone(&trace), TestTx(Arc::clone(&trace)))),
        dma,
    )
    .unwrap();
    let mut group = device.poll_groups.pop().unwrap();
    group.rx.initial_refill(2).unwrap();
    // Hold the third preallocated pool token so the next replacement reaches
    // the allocator. Only this device's allocator can fail in this test.
    let spare = group.rx.allocate_replacement().unwrap();
    let (rx_ready, mut received) = spsc_ring(2);
    let (recycle, rx_recycle) = spsc_ring(2);
    let (mut transmit, tx_ready) = spsc_ring(2);
    let (tx_free, _free) = spsc_ring(2);
    let shared = Arc::new(PollGroupState::new(
        test_identity(),
        Arc::new(QueueNotification::new()),
    ));
    shared.activate(false);
    let mut executor = QueueGroupExecutor {
        wifi_startup_group: None,
        group,
        rx_ready,
        rx_recycle,
        rx_recycler: Arc::new(RxRecycler::new(recycle, Arc::clone(&shared), 2)),
        rx_spares: Vec::new(),
        rx_extra_buffers: 0,
        tx_ready,
        tx_free,
        pending_rx: None,
        pending_rx_refill: VecDeque::with_capacity(2),
        pending_tx: None,
        pending_tx_free: None,
        retry_at: None,
        shared,
    };
    DMA.0.store(true, Ordering::Relaxed);
    let outcome = executor.poll(1);
    DMA.0.store(false, Ordering::Relaxed);
    assert!(
        !matches!(outcome, GroupPollOutcome::Failed(_)),
        "temporary DMA allocation failure permanently disabled RX and TX"
    );
    assert!(
        received.pop().is_none(),
        "packet escaped without a replacement"
    );

    drop(spare);
    let buffer = executor.group.tx_pool.allocate(60).unwrap();
    assert!(
        transmit
            .push(TxRequest {
                buffer,
                options: TxSubmitOptions {
                    notify: TxNotify::Deferred,
                    ..Default::default()
                }
            })
            .is_ok()
    );
    assert!(matches!(executor.poll(256), GroupPollOutcome::More(_)));
    assert!(matches!(executor.poll(256), GroupPollOutcome::Idle(_)));
    let packet = received
        .pop()
        .expect("RX must resume after allocation recovers");
    packet
        .buffer
        .read_with_cpu(60, |bytes| assert_eq!(bytes, &[0; 60]));
    assert!(
        received.pop().is_none(),
        "the packet dropped under memory pressure was delivered"
    );
    assert_eq!(
        &*trace.lock().unwrap(),
        &["rx", "tx", "flush", "retry", "rx", "refill", "refill"]
    );
    assert!(executor.pending_rx_refill.is_empty());
    assert_eq!(executor.shared.take_pending_rx_drops(), 1);
    assert_eq!(executor.shared.take_pending_rx_drops(), 0);

    let limit = executor.group.rx.capacity().max(QUEUE_BUDGET);
    let mut held = Vec::new();
    while executor.rx_extra_buffers < limit {
        held.push(executor.take_rx_replacement().unwrap());
    }
    assert!(
        executor.take_rx_replacement().is_none(),
        "detached RX tokens exceeded the queue budget"
    );
    let recycled = held.pop().unwrap();
    let address = recycled.read_with_cpu(1, |bytes| bytes.as_ptr() as usize);
    executor.rx_recycler.recycle(recycled);
    executor
        .rx_recycler
        .drain_into(&mut executor.rx_recycle, &mut executor.rx_spares, 1);
    let reused = executor
        .take_rx_replacement()
        .expect("recycled tokens remain usable at the limit");
    assert_eq!(
        reused.read_with_cpu(1, |bytes| bytes.as_ptr() as usize),
        address
    );
    assert_eq!(executor.rx_extra_buffers, limit);
}

fn queue_config() -> QueueConfig {
    QueueConfig {
        ring_size: 3,
        buf_size: 2048,
        align: 64,
        dma_mask: u64::MAX,
    }
}

struct TestTx(Trace);

impl ITxQueue for TestTx {
    fn id(&self) -> NetQueueId {
        NetQueueId::new(0)
    }
    fn config(&self) -> QueueConfig {
        queue_config()
    }
    fn submit(&mut self, _buffer: DmaBuffer) -> Result<(), SubmitError> {
        self.0.lock().unwrap().push("tx");
        Ok(())
    }
    fn flush(&mut self) {
        self.0.lock().unwrap().push("flush");
    }
    fn reclaim(&mut self) -> Option<DmaBuffer> {
        None
    }
}

struct TestRx {
    trace: Trace,
    completions: VecDeque<RxCompletion>,
    initial: usize,
    reclaimed: usize,
    replacements: Vec<DmaBuffer>,
}

impl IRxQueue for TestRx {
    fn id(&self) -> NetQueueId {
        NetQueueId::new(0)
    }
    fn config(&self) -> QueueConfig {
        queue_config()
    }
    fn submit(&mut self, mut buffer: DmaBuffer) -> Result<(), SubmitError> {
        if self.initial > 0 {
            self.initial -= 1;
            buffer.write_with_cpu(|packet| packet.fill(self.initial as u8));
            self.completions.push_back(RxCompletion {
                buffer,
                packet_len: 60,
            });
            return Ok(());
        }
        // Model a software queue whose owner cannot accept more buffers
        // until both completion slots have been consumed.
        if self.reclaimed < 2 {
            self.trace.lock().unwrap().push("retry");
            return Err(SubmitError::new(buffer, NetError::Retry));
        }
        self.trace.lock().unwrap().push("refill");
        self.replacements.push(buffer);
        Ok(())
    }
    fn reclaim(&mut self) -> Option<RxCompletion> {
        let completion = self.completions.pop_front()?;
        self.reclaimed += 1;
        self.trace.lock().unwrap().push("rx");
        Some(completion)
    }
}

struct TestIrq;
impl NetHardIrqHandler for TestIrq {
    fn handle_irq(&mut self) -> NetHardIrqResult {
        NetHardIrqResult::Spurious
    }
}

struct MissingDeviceStartup {
    cancel_attempted: Arc<AtomicBool>,
    cancel_fails: bool,
}

impl NetOwnerStartup for MissingDeviceStartup {
    fn start(&mut self, _now_nanos: u64) -> Result<NetOwnerStartupProgress, NetError> {
        Err(NetError::DeviceNotPresent)
    }

    fn advance(&mut self, _now_nanos: u64) -> Result<NetOwnerStartupProgress, NetError> {
        panic!("a missing device must not advance startup")
    }

    fn cancel(&mut self) -> Result<(), NetError> {
        self.cancel_attempted.store(true, Ordering::Release);
        if self.cancel_fails {
            Err(NetError::InvalidParts)
        } else {
            Ok(())
        }
    }
}
impl NetPollIrqControl for TestIrq {
    fn quiesce(&mut self) -> Result<(), NetError> {
        Ok(())
    }
    fn shutdown(&mut self) -> Result<(), NetError> {
        Ok(())
    }
    fn rearm_and_check(&mut self, _now_nanos: u64) -> Result<NetRearmResult, NetError> {
        Ok(NetRearmResult::Idle)
    }
}

struct TestDevice<T>(Trace, T);
impl<T: ITxQueue> rd_net::DriverGeneric for TestDevice<T> {
    fn name(&self) -> &str {
        "test"
    }
}
impl<T: ITxQueue + 'static> NetDevice for TestDevice<T> {
    fn into_parts(self: Box<Self>) -> Result<NetDeviceParts, NetError> {
        Ok(NetDeviceParts {
            info: NetDeviceInfo::new("test", [0; 6]),
            control: Box::new(FixedNetControl::new([0; 6])),
            wifi_control: None,
            poll_groups: vec![NetPollGroupParts {
                id: NetPollGroupId::new(0),
                queues: NetQueuePairParts {
                    tx: Box::new(self.1),
                    rx: Box::new(TestRx {
                        trace: self.0,
                        completions: VecDeque::new(),
                        initial: 2,
                        reclaimed: 0,
                        replacements: Vec::new(),
                    }),
                },
                irq_control: Box::new(TestIrq),
                owner_startup: None,
                irq_endpoints: vec![NetHardIrqEndpoint::new(
                    NetIrqSourceId::new(0),
                    Box::new(TestIrq),
                )],
            }],
        })
    }
}

#[test]
fn missing_device_startup_is_cancelled_without_publishing_queues() {
    for cancel_fails in [false, true] {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let dma = DeviceDma::new(
            DmaDeviceInfo::new(
                DmaDomainId::Direct,
                DmaCoherency::Coherent,
                DmaConstraints::new(u64::MAX),
            ),
            &TEST_DMA,
        );
        let mut device =
            rd_net::prepare_device(Box::new(TestDevice(Arc::clone(&trace), TestTx(trace))), dma)
                .unwrap();
        let mut group = device.poll_groups.pop().unwrap();
        let cancel_attempted = Arc::new(AtomicBool::new(false));
        group.owner_startup = Some(Box::new(MissingDeviceStartup {
            cancel_attempted: Arc::clone(&cancel_attempted),
            cancel_fails,
        }));

        let (rx_ready, _protocol_rx) = spsc_ring(2);
        let (rx_recycle, recycle) = spsc_ring(2);
        let (tx_free, mut protocol_tx_free) = spsc_ring(2);
        let (_protocol_tx, tx_ready) = spsc_ring(2);
        let shared = Arc::new(PollGroupState::new(
            test_identity(),
            Arc::new(QueueNotification::new()),
        ));
        let mut executor = QueueGroupExecutor {
            wifi_startup_group: None,
            group,
            rx_ready,
            rx_recycle: recycle,
            rx_recycler: Arc::new(RxRecycler::new(rx_recycle, Arc::clone(&shared), 2)),
            rx_spares: Vec::new(),
            rx_extra_buffers: 0,
            tx_ready,
            tx_free,
            pending_rx: None,
            pending_rx_refill: VecDeque::with_capacity(2),
            pending_tx: None,
            pending_tx_free: None,
            retry_at: None,
            shared: Arc::clone(&shared),
        };

        let result = executor.initialize(|| panic!("absent device startup must not wait"));
        if cancel_fails {
            assert!(matches!(result, Err(NetError::InvalidParts)));
        } else {
            assert!(result.is_ok());
        }
        assert!(cancel_attempted.load(Ordering::Acquire));
        assert_eq!(shared.startup_absent(), !cancel_fails);
        assert!(shared.is_disabled());
        assert_eq!(executor.group.rx.posted(), 0);
        assert!(protocol_tx_free.pop().is_none());
    }
}

struct StartupWifi(Arc<AtomicBool>);

impl Drop for StartupWifi {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl rd_net::WifiControl for StartupWifi {
    fn start(
        &mut self,
        _operation: &rd_net::WifiOperation,
        _now_nanos: u64,
    ) -> Result<rd_net::WifiControlProgress, NetError> {
        panic!("Wi-Fi transactions must not start before publication")
    }

    fn advance(&mut self, _now_nanos: u64) -> Result<rd_net::WifiControlProgress, NetError> {
        panic!("Wi-Fi transactions must not advance before publication")
    }

    fn cancel(&mut self) -> Result<(), NetError> {
        panic!("no Wi-Fi transaction is active before publication")
    }

    fn startup_transaction(&self) -> Option<rd_net::WifiTransaction> {
        None
    }
}

fn startup_executor(absent: bool, trace: Trace) -> QueueGroupExecutor {
    let dma = DeviceDma::new(
        DmaDeviceInfo::new(
            DmaDomainId::Direct,
            DmaCoherency::Coherent,
            DmaConstraints::new(u64::MAX),
        ),
        &TEST_DMA,
    );
    let mut device =
        rd_net::prepare_device(Box::new(TestDevice(Arc::clone(&trace), TestTx(trace))), dma)
            .unwrap();
    let group = device.poll_groups.pop().unwrap();
    let (rx_ready, _protocol_rx) = spsc_ring(2);
    let (rx_recycle, recycle) = spsc_ring(2);
    let (tx_free, _protocol_tx_free) = spsc_ring(2);
    let (_protocol_tx, tx_ready) = spsc_ring(2);
    let shared = Arc::new(PollGroupState::new(
        test_identity(),
        Arc::new(QueueNotification::new()),
    ));
    let mut executor = QueueGroupExecutor {
        group,
        wifi_startup_group: None,
        rx_ready,
        rx_recycle: recycle,
        rx_recycler: Arc::new(RxRecycler::new(rx_recycle, Arc::clone(&shared), 2)),
        rx_spares: Vec::new(),
        rx_extra_buffers: 0,
        tx_ready,
        tx_free,
        pending_rx: None,
        pending_rx_refill: VecDeque::with_capacity(2),
        pending_tx: None,
        pending_tx_free: None,
        retry_at: None,
        shared,
    };
    if absent {
        executor.group.owner_startup = Some(Box::new(MissingDeviceStartup {
            cancel_attempted: Arc::new(AtomicBool::new(false)),
            cancel_fails: false,
        }));
        executor
            .initialize(|| panic!("absent startup must not wait"))
            .unwrap();
    }
    executor
}

#[test]
fn startup_pruning_releases_absent_queues_and_remaps_wifi_slots() {
    for absent in [
        vec![true, false, true, false],
        vec![true],
        vec![false],
        vec![],
    ] {
        let mut groups = Vec::new();
        let mut wifi = Vec::new();
        let mut queue_owners = Vec::new();
        let mut wifi_dropped = Vec::new();
        let mut states = Vec::new();
        for (group_index, &missing) in absent.iter().enumerate() {
            let trace = Arc::new(Mutex::new(Vec::new()));
            queue_owners.push(Arc::downgrade(&trace));
            let executor = startup_executor(missing, trace);
            states.push(Arc::clone(&executor.shared));
            groups.push(executor);
            let dropped = Arc::new(AtomicBool::new(false));
            wifi_dropped.push(Arc::clone(&dropped));
            wifi.push(WifiExecutorSlot {
                group_index,
                control: Box::new(StartupWifi(dropped)),
                queue: Arc::new(crate::queue_runtime::WifiControlQueue::new()),
                active: None,
            });
        }

        for command in [
            crate::queue_runtime::COMMAND_START,
            COMMAND_QUARANTINE,
            COMMAND_STOP,
        ] {
            assert_eq!(
                retain_started_executor_groups(&mut groups, &mut wifi, command),
                None
            );
            assert!(queue_owners.iter().all(|owner| owner.upgrade().is_some()));
            assert!(
                wifi_dropped
                    .iter()
                    .all(|dropped| !dropped.load(Ordering::Acquire))
            );
        }
        let status = retain_started_executor_groups(&mut groups, &mut wifi, COMMAND_RUN);

        let mut published = 0;
        for (index, &missing) in absent.iter().enumerate() {
            assert_eq!(queue_owners[index].upgrade().is_none(), missing);
            assert_eq!(wifi_dropped[index].load(Ordering::Acquire), missing);
            if !missing {
                assert_eq!(wifi[published].group_index, published);
                assert!(Arc::ptr_eq(&groups[published].shared, &states[index]));
                published += 1;
            }
        }
        assert_eq!(groups.len(), published);
        assert_eq!(wifi.len(), published);
        assert_eq!(
            status,
            Some(if published == 0 {
                STATUS_EMPTY
            } else {
                STATUS_READY
            })
        );
    }
}

#[test]
fn absent_wifi_control_stops_surviving_device_group() {
    let control = startup_executor(true, Arc::new(Mutex::new(Vec::new())));
    let mut sibling = startup_executor(false, Arc::new(Mutex::new(Vec::new())));
    sibling
        .initialize(|| panic!("sibling startup must not wait"))
        .unwrap();
    assert!(!sibling.shared.is_disabled());
    sibling.wifi_startup_group = Some(Arc::clone(&control.shared));
    sibling.stop_if_wifi_absent().unwrap();
    assert!(sibling.shared.startup_absent());
    assert!(sibling.shared.is_disabled());
    let (mut port, ..) = crate::queue_runtime::tests::tx_test_port(TxQueueDiscipline::NoQueue, 0);
    let (mut sibling_port, ..) =
        crate::queue_runtime::tests::tx_test_port(TxQueueDiscipline::NoQueue, 0);
    port.groups[0].shared = Arc::clone(&control.shared);
    sibling_port.groups[0].shared = Arc::clone(&sibling.shared);
    port.groups.append(&mut sibling_port.groups);
    let (ports, indices) = crate::queue_runtime::retain_started_ports(vec![port]);
    assert!(ports.is_empty());
    assert_eq!(indices, vec![None]);
    let dropped = Arc::new(AtomicBool::new(false));
    let mut wifi = vec![WifiExecutorSlot {
        group_index: 0,
        control: Box::new(StartupWifi(Arc::clone(&dropped))),
        queue: Arc::new(crate::queue_runtime::WifiControlQueue::new()),
        active: None,
    }];
    let mut groups = vec![control, sibling];
    assert_eq!(
        retain_started_executor_groups(&mut groups, &mut wifi, COMMAND_RUN),
        Some(STATUS_EMPTY)
    );
    assert!(groups.is_empty());
    assert!(wifi.is_empty());
    assert!(dropped.load(Ordering::Acquire));
}

struct PruneIrq {
    trace: Trace,
    fail_shutdown: bool,
}

impl NetPollIrqControl for PruneIrq {
    fn quiesce(&mut self) -> Result<(), NetError> {
        self.trace.lock().unwrap().push("quiesce");
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), NetError> {
        self.trace.lock().unwrap().push("shutdown");
        if self.fail_shutdown {
            Err(NetError::InvalidParts)
        } else {
            Ok(())
        }
    }

    fn rearm_and_check(&mut self, _now_nanos: u64) -> Result<NetRearmResult, NetError> {
        Ok(NetRearmResult::Idle)
    }
}

#[test]
fn wifi_device_pruning_requires_shutdown_and_preserves_unrelated_groups() {
    for missing in [false, true] {
        for fail_shutdown in [false, true] {
            let control = startup_executor(missing, Arc::new(Mutex::new(Vec::new())));
            let trace = Arc::new(Mutex::new(Vec::new()));
            let mut sibling = startup_executor(false, Arc::new(Mutex::new(Vec::new())));
            sibling
                .initialize(|| panic!("sibling startup must not wait"))
                .unwrap();
            sibling.wifi_startup_group = Some(Arc::clone(&control.shared));
            sibling.group.irq_control = Box::new(PruneIrq {
                trace: Arc::clone(&trace),
                fail_shutdown,
            });
            assert_eq!(
                sibling.stop_if_wifi_absent().is_err(),
                missing && fail_shutdown
            );
            assert_eq!(sibling.shared.startup_absent(), missing && !fail_shutdown);
            assert_eq!(
                *trace.lock().unwrap(),
                if missing {
                    vec!["quiesce", "shutdown"]
                } else {
                    vec![]
                }
            );
            if missing && !fail_shutdown {
                sibling.stop_if_wifi_absent().unwrap();
                assert_eq!(*trace.lock().unwrap(), vec!["quiesce", "shutdown"]);
            }
            let mut unrelated = startup_executor(false, Arc::new(Mutex::new(Vec::new())));
            unrelated.stop_if_wifi_absent().unwrap();
            assert!(!unrelated.shared.startup_absent());
        }
    }
}

#[test]
fn rx_refill_retry_drains_completions_and_preserves_tx_flush() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let dma = DeviceDma::new(
        DmaDeviceInfo::new(
            DmaDomainId::Direct,
            DmaCoherency::Coherent,
            DmaConstraints::new(u64::MAX),
        ),
        &TEST_DMA,
    );
    let mut device = rd_net::prepare_device(
        Box::new(TestDevice(Arc::clone(&trace), TestTx(Arc::clone(&trace)))),
        dma,
    )
    .unwrap();
    let mut group = device.poll_groups.pop().unwrap();
    group.rx.initial_refill(2).unwrap();
    let (rx_ready, mut received) = spsc_ring(1);
    let (recycle, rx_recycle) = spsc_ring(2);
    let (mut transmit, tx_ready) = spsc_ring(2);
    let (tx_free, _free) = spsc_ring(2);
    let shared = Arc::new(PollGroupState::new(
        test_identity(),
        Arc::new(QueueNotification::new()),
    ));
    shared.activate(false);
    let buffer = group.tx_pool.allocate(60).unwrap();
    assert!(
        transmit
            .push(TxRequest {
                buffer,
                options: TxSubmitOptions {
                    notify: TxNotify::Deferred,
                    ..Default::default()
                },
            })
            .is_ok()
    );
    let mut executor = QueueGroupExecutor {
        group,
        wifi_startup_group: None,
        rx_ready,
        rx_recycle,
        rx_recycler: Arc::new(RxRecycler::new(recycle, Arc::clone(&shared), 2)),
        rx_spares: Vec::new(),
        rx_extra_buffers: 0,
        tx_ready,
        tx_free,
        pending_rx: None,
        pending_rx_refill: VecDeque::with_capacity(2),
        pending_tx: None,
        pending_tx_free: None,
        retry_at: None,
        shared,
    };

    assert!(matches!(executor.poll(2), GroupPollOutcome::More(2)));
    assert!(matches!(executor.poll(256), GroupPollOutcome::More(_)));
    assert_eq!(
        &*trace.lock().unwrap(),
        &["tx", "flush", "rx", "retry", "rx"]
    );
    assert!(
        received.pop().is_none(),
        "RX escaped before replacement was accepted"
    );
    assert_eq!(executor.pending_rx_refill.len(), 2);

    assert!(matches!(executor.poll(256), GroupPollOutcome::Blocked(_)));
    let first = received.pop().unwrap();
    first
        .buffer
        .read_with_cpu(60, |packet| assert_eq!(packet, &[1; 60]));
    assert!(executor.pending_rx.is_some());
    assert!(matches!(executor.poll(256), GroupPollOutcome::Idle(_)));
    let second = received.pop().unwrap();
    second
        .buffer
        .read_with_cpu(60, |packet| assert_eq!(packet, &[0; 60]));
    assert!(executor.pending_rx.is_none());
    assert!(executor.pending_rx_refill.is_empty());
    assert_eq!(
        trace
            .lock()
            .unwrap()
            .iter()
            .filter(|&&event| event == "refill")
            .count(),
        2
    );
}

struct GatedTx {
    blocked: Arc<AtomicBool>,
    packets: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ITxQueue for GatedTx {
    fn id(&self) -> NetQueueId {
        NetQueueId::new(0)
    }

    fn config(&self) -> QueueConfig {
        queue_config()
    }

    fn submit(&mut self, buffer: DmaBuffer) -> Result<(), SubmitError> {
        if self.blocked.load(Ordering::Relaxed) {
            return Err(SubmitError::new(buffer, NetError::Retry));
        }
        buffer.read_with_cpu(buffer.len(), |packet| {
            self.packets.lock().unwrap().push(packet.to_vec());
        });
        Ok(())
    }

    fn reclaim(&mut self) -> Option<DmaBuffer> {
        None
    }
}

#[test]
fn tx_backpressure_allows_rx_delivery_before_tx_resumes() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let blocked = Arc::new(AtomicBool::new(true));
    let packets = Arc::new(Mutex::new(Vec::new()));
    let dma = DeviceDma::new(
        DmaDeviceInfo::new(
            DmaDomainId::Direct,
            DmaCoherency::Coherent,
            DmaConstraints::new(u64::MAX),
        ),
        &TEST_DMA,
    );
    let mut device = rd_net::prepare_device(
        Box::new(TestDevice(
            trace,
            GatedTx {
                blocked: Arc::clone(&blocked),
                packets: Arc::clone(&packets),
            },
        )),
        dma,
    )
    .unwrap();
    let mut group = device.poll_groups.pop().unwrap();
    group.rx.initial_refill(2).unwrap();
    let (rx_ready, mut received) = spsc_ring(2);
    let (recycle, rx_recycle) = spsc_ring(2);
    let (mut transmit, tx_ready) = spsc_ring(2);
    let (tx_free, _free) = spsc_ring(2);
    let shared = Arc::new(PollGroupState::new(
        test_identity(),
        Arc::new(QueueNotification::new()),
    ));
    shared.activate(false);
    for byte in [0xa5, 0x5a] {
        let mut buffer = group.tx_pool.allocate(60).unwrap();
        buffer.write_with_cpu(|packet| packet.fill(byte));
        assert!(
            transmit
                .push(TxRequest {
                    buffer,
                    options: TxSubmitOptions::default(),
                })
                .is_ok()
        );
    }
    let mut executor = QueueGroupExecutor {
        group,
        wifi_startup_group: None,
        rx_ready,
        rx_recycle,
        rx_recycler: Arc::new(RxRecycler::new(recycle, Arc::clone(&shared), 2)),
        rx_spares: Vec::new(),
        rx_extra_buffers: 0,
        tx_ready,
        tx_free,
        pending_rx: None,
        pending_rx_refill: VecDeque::with_capacity(2),
        pending_tx: None,
        pending_tx_free: None,
        retry_at: None,
        shared,
    };
    // A software-backed NIC may need its completed RX slots drained before
    // the common owner can finish outstanding TX. Keep TX blocked until RX
    // delivery is proven, rather than relying on an IRQ or a timed retry.
    executor.poll(256);
    executor.poll(256);
    for byte in [1, 0] {
        let completion = received
            .pop()
            .expect("TX Retry starved a completed RX packet");
        completion
            .buffer
            .read_with_cpu(60, |packet| assert_eq!(packet, &[byte; 60]));
    }
    assert!(packets.lock().unwrap().is_empty());
    assert!(
        matches!(executor.poll(256), GroupPollOutcome::Idle(_)),
        "a still-blocked TX must rearm instead of busy-polling"
    );
    blocked.store(false, Ordering::Relaxed);
    executor.poll(256);
    executor.poll(256);
    assert_eq!(
        *packets.lock().unwrap(),
        vec![vec![0xa5; 60], vec![0x5a; 60]]
    );
}

/// RX queue that rejects replacements with a non-retryable error, so a round
/// that already completed work can end in `Failed`.
struct HardFailingRx {
    trace: Trace,
    completions: VecDeque<RxCompletion>,
    initial: usize,
}

impl IRxQueue for HardFailingRx {
    fn id(&self) -> NetQueueId {
        NetQueueId::new(0)
    }
    fn config(&self) -> QueueConfig {
        queue_config()
    }
    fn submit(&mut self, mut buffer: DmaBuffer) -> Result<(), SubmitError> {
        if self.initial > 0 {
            self.initial -= 1;
            buffer.write_with_cpu(|packet| packet.fill(self.initial as u8));
            self.completions.push_back(RxCompletion {
                buffer,
                packet_len: 60,
            });
            return Ok(());
        }
        Err(SubmitError::new(buffer, NetError::LinkDown))
    }
    fn reclaim(&mut self) -> Option<RxCompletion> {
        let completion = self.completions.pop_front()?;
        self.trace.lock().unwrap().push("rx");
        Some(completion)
    }
}

struct HardFailingRxDevice(Trace);

impl rd_net::DriverGeneric for HardFailingRxDevice {
    fn name(&self) -> &str {
        "test-hard-failing-rx"
    }
}

impl NetDevice for HardFailingRxDevice {
    fn into_parts(self: Box<Self>) -> Result<NetDeviceParts, NetError> {
        Ok(NetDeviceParts {
            info: NetDeviceInfo::new("test-hard-failing-rx", [0; 6]),
            control: Box::new(FixedNetControl::new([0; 6])),
            wifi_control: None,
            poll_groups: vec![NetPollGroupParts {
                id: NetPollGroupId::new(0),
                queues: NetQueuePairParts {
                    tx: Box::new(TestTx(Arc::clone(&self.0))),
                    rx: Box::new(HardFailingRx {
                        trace: self.0,
                        completions: VecDeque::new(),
                        initial: 1,
                    }),
                },
                irq_control: Box::new(TestIrq),
                owner_startup: None,
                irq_endpoints: vec![NetHardIrqEndpoint::new(
                    NetIrqSourceId::new(0),
                    Box::new(TestIrq),
                )],
            }],
        })
    }
}

/// Builds an executor whose RX queue rejects replacements permanently, under
/// its own identity so its reports can be told apart from concurrent tests.
fn port_test_executor(
    identity: NetQueueIdentity,
    trace: Trace,
    tx_ready: SpscConsumer<TxRequest>,
) -> QueueGroupExecutor {
    let dma = DeviceDma::new(
        DmaDeviceInfo::new(
            DmaDomainId::Direct,
            DmaCoherency::Coherent,
            DmaConstraints::new(u64::MAX),
        ),
        &TEST_DMA,
    );
    let mut device =
        rd_net::prepare_device(Box::new(HardFailingRxDevice(Arc::clone(&trace))), dma).unwrap();
    let mut group = device.poll_groups.pop().unwrap();
    // No initial refill and no queued completions: a plain round neither
    // reclaims nor refills, so its outcome depends only on the test's input.
    group.rx.initial_refill(0).unwrap();
    let (rx_ready, _protocol_rx) = spsc_ring(2);
    let (rx_recycle, recycle) = spsc_ring(2);
    let (tx_free, _protocol_tx_free) = spsc_ring(2);
    let shared = Arc::new(PollGroupState::new(
        identity,
        Arc::new(QueueNotification::new()),
    ));
    shared.activate(false);
    QueueGroupExecutor {
        group,
        wifi_startup_group: None,
        rx_ready,
        rx_recycle: recycle,
        rx_recycler: Arc::new(RxRecycler::new(rx_recycle, Arc::clone(&shared), 2)),
        rx_spares: Vec::new(),
        rx_extra_buffers: 0,
        tx_ready,
        tx_free,
        pending_rx: None,
        pending_rx_refill: VecDeque::with_capacity(2),
        pending_tx: None,
        pending_tx_free: None,
        retry_at: None,
        shared,
    }
}

// ---- queue poll observation port ----

/// The observation port is process-wide, so the tests below share one observer
/// function (installing the same function is idempotent, replacing a live one
/// is not), serialize on one lock, and filter captured reports by their own
/// identity: other tests in this binary keep polling while the gate is open.
static POLL_REPORTS: Mutex<Vec<QueuePollReport>> = Mutex::new(Vec::new());
static POLL_PORT_LOCK: Mutex<()> = Mutex::new(());

fn capture_poll_report(report: QueuePollReport) {
    POLL_REPORTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(report);
}

fn begin_poll_capture(enabled: bool) -> std::sync::MutexGuard<'static, ()> {
    let guard = POLL_PORT_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    install_queue_poll_observer(capture_poll_report);
    publish_queue_poll_gate(enabled);
    POLL_REPORTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
    guard
}

fn reports_of(identity: NetQueueIdentity) -> Vec<QueuePollReport> {
    POLL_REPORTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .filter(|report| report.identity == identity)
        .copied()
        .collect()
}

/// Identity reserved for one port test, so reports from other tests that poll
/// concurrently cannot be mistaken for its own.
fn port_identity(tag: usize) -> NetQueueIdentity {
    NetQueueIdentity {
        discovery_order: 900 + tag,
        group_id: NetPollGroupId::new(7),
        owner_cpu: 3,
    }
}

#[test]
fn queue_poll_reports_exactly_once_per_round_with_its_identity_and_budget() {
    let _port = begin_poll_capture(true);
    let identity = port_identity(1);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (mut transmit, tx_ready) = spsc_ring(2);
    let mut executor = port_test_executor(identity, Arc::clone(&trace), tx_ready);

    let outcome = executor.poll(256);
    assert_eq!(outcome, GroupPollOutcome::Idle(0));
    let reports = reports_of(identity);
    assert_eq!(reports.len(), 1, "one round must report exactly once");
    assert_eq!(reports[0].budget, 256);
    assert_eq!(reports[0].work_units, 0);
    assert_eq!(reports[0].outcome, QueuePollOutcome::Idle);

    // A second round reports again, and still only once.
    let buffer = executor.group.tx_pool.allocate(60).unwrap();
    assert!(
        transmit
            .push(TxRequest {
                buffer,
                options: TxSubmitOptions {
                    notify: TxNotify::Deferred,
                    ..Default::default()
                },
            })
            .is_ok()
    );
    let outcome = executor.poll(64);
    assert_eq!(outcome, GroupPollOutcome::Idle(1));
    let reports = reports_of(identity);
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[1].budget, 64);
    assert_eq!(reports[1].work_units, 1);
    publish_queue_poll_gate(false);
}

#[test]
fn queue_poll_reports_the_work_done_before_a_failed_round() {
    let _port = begin_poll_capture(true);
    let identity = port_identity(2);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (mut transmit, tx_ready) = spsc_ring(2);
    let mut executor = port_test_executor(identity, Arc::clone(&trace), tx_ready);

    // One submitted frame and one pending refill that the device rejects
    // permanently: the round does work and then fails.
    let buffer = executor.group.tx_pool.allocate(60).unwrap();
    assert!(
        transmit
            .push(TxRequest {
                buffer,
                options: TxSubmitOptions {
                    notify: TxNotify::Deferred,
                    ..Default::default()
                },
            })
            .is_ok()
    );
    let replacement = executor.group.rx.allocate_replacement().unwrap();
    executor.pending_rx_refill.push_back(PendingRxRefill {
        completion: None,
        replacement,
    });

    let outcome = executor.poll(256);
    let work = outcome.work();
    assert!(
        matches!(outcome, GroupPollOutcome::Failed(_)),
        "a permanent RX refill error must fail the round"
    );
    assert!(work > 0, "the round submitted a frame before failing");
    let reports = reports_of(identity);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, QueuePollOutcome::Failed);
    assert_eq!(
        reports[0].work_units, work,
        "a failed round must report the work it completed"
    );
    publish_queue_poll_gate(false);
}

#[test]
fn queue_poll_outcome_is_unchanged_by_the_observation_port() {
    let identity = port_identity(3);
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (_tx, tx_ready) = spsc_ring(2);
    let mut executor = port_test_executor(identity, Arc::clone(&trace), tx_ready);

    let _port = begin_poll_capture(false);
    let closed = executor.poll(256);
    assert!(
        reports_of(identity).is_empty(),
        "a closed gate must not report"
    );

    publish_queue_poll_gate(true);
    let open = executor.poll(256);
    assert_eq!(reports_of(identity).len(), 1);

    assert_eq!(closed, open, "observation must not change the poll outcome");
    publish_queue_poll_gate(false);
}
