use atomic_bitfield::AtomicBitField;
use bitflags::_core::ops::RangeInclusive;
use core::ops::Deref;
use core::sync::atomic::{AtomicU8, Ordering};
use spin::{Mutex, MutexGuard, Once};
use x86_64::structures::tss::TaskStateSegment;

pub static TSS: Once<Tss> = Once::new();

// "avoid placing a page boundary in the first 104 bytes"
#[repr(C, align(4096))]
pub struct Tss {
    pub tss: TaskStateSegment,
    pub iomap: IopbLock,
}

impl Tss {
    pub fn new(tss: TaskStateSegment) -> Self {
        let mut tss = Tss {
            tss,
            iomap: IopbLock::default(),
        };

        // Absolute values don't matter, only the difference
        let tss_addr = (&tss.tss) as *const _ as usize;
        let iomap_addr = (&tss.iomap) as *const _ as usize;
        let iomap_base = (iomap_addr - tss_addr) as u16;

        tss.tss.iomap_base = iomap_base;
        tss
    }
}

#[repr(C)]
pub struct IoPermissionsBitMap {
    iomap: [AtomicU8; 8192],
    /// Signifies end of iomap
    _always_ff: u8,
}

impl IoPermissionsBitMap {
    fn new(iomap: [AtomicU8; 8192]) -> IoPermissionsBitMap {
        IoPermissionsBitMap {
            iomap,
            _always_ff: 0xff,
        }
    }

    pub fn set_port_range_usable(&self, ports: RangeInclusive<u16>, usable: bool) {
        assert!(
            ports.end() / 8 < 8192,
            "Port 0x{:x} out of bounds",
            ports.end()
        );

        for port in ports {
            let byte_idx = port / 8;
            let bit = port % 8;
            // For some reason 1 = disabled
            self.iomap[byte_idx as usize].swap_bit(bit as usize, !usable, Ordering::Release);
        }
    }

    pub fn set_port_usable(&self, port: u16, usable: bool) {
        assert!(port / 8 < 8192, "Port 0x{:x} out of bounds", port);

        let byte_idx = port / 8;
        let bit = port % 8;
        // For some reason 1 = disabled
        self.iomap[byte_idx as usize].swap_bit(bit as usize, !usable, Ordering::Release);
    }

    pub fn set_ports_usable(&self, ports: &[u16], usable: bool) {
        for port in ports {
            self.set_port_usable(*port, usable);
        }
    }

    pub fn is_port_usable(&self, port: u16) -> bool {
        assert!(port / 8 < 8192, "Port 0x{:x} out of bounds", port);

        let byte_idx = port / 8;
        let bit = port % 8;
        // For some reason 1 = disabled
        !self.iomap[byte_idx as usize].get_bit(bit as usize, Ordering::Acquire)
    }
}

impl Default for IoPermissionsBitMap {
    fn default() -> IoPermissionsBitMap {
        #[allow(clippy::declare_interior_mutable_const)] // Used for array init
        const UNUSABLE: AtomicU8 = AtomicU8::new(0xff); // 0b1 = cannot use port
        IoPermissionsBitMap::new([UNUSABLE; 8192])
    }
}

#[derive(Default)]
#[repr(C)]
pub struct IopbLock {
    iopb: IoPermissionsBitMap,
    lock: Mutex<()>,
}

impl IopbLock {
    pub fn lock_or_panic(&self) -> IopbLockGuard<'_> {
        IopbLockGuard {
            iopb: &self.iopb,
            _guard: self
                .lock
                .try_lock()
                .expect("IO permissions bitmap concurrently locked!"),
        }
    }

    /// # Safety
    ///
    /// Do not concurrently modify the io bitmap from elsewhere. It's fine if the CPU reads it.
    pub unsafe fn as_slice(&self) -> &[u8] {
        // SAFETY: repr(C) and enough provenance
        core::slice::from_raw_parts(self as *const _ as *const _, 8193)
    }
}

pub struct IopbLockGuard<'a> {
    iopb: &'a IoPermissionsBitMap,
    _guard: MutexGuard<'a, ()>,
}

impl Deref for IopbLockGuard<'_> {
    type Target = IoPermissionsBitMap;

    fn deref(&self) -> &IoPermissionsBitMap {
        &self.iopb
    }
}
