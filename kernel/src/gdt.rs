use crate::tss::TSS;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const PANICKING_EXCEPTION_IST_INDEX: u16 = 1;
pub const IRQ_IST_INDEX: u16 = 2;
pub const NMI_IST_INDEX: u16 = 3;

use x86_64::structures::gdt::{DescriptorFlags as Flags, *};

lazy_static::lazy_static! {
    pub static ref GDT: Gdt = {
        let mut gdt = GlobalDescriptorTable::new();

        let tss = TSS.wait().unwrap();
        let tss = gdt.add_entry(
            Descriptor::tss_segment_with_iomap(&tss.tss, unsafe { tss.iomap.as_slice() })
                .unwrap()
        );

        let kernel_cs = gdt.add_entry(Descriptor::kernel_code_segment());
        let kernel_ds = gdt.add_entry(Descriptor::UserSegment(
            (Flags::USER_SEGMENT | Flags::PRESENT).bits() | (1 << 41),
        ));

        let user_ds = gdt.add_entry(Descriptor::UserSegment( // RW bit & ring3
            (Flags::USER_SEGMENT | Flags::PRESENT | Flags::DPL_RING_3 | Flags::WRITABLE).bits()
        ));
        let user_cs = gdt.add_entry(Descriptor::UserSegment(
            (Flags::USER_SEGMENT | Flags::PRESENT | Flags::EXECUTABLE | Flags::LONG_MODE | Flags::DPL_RING_3).bits()
        ));

        Gdt {
            table: gdt,
            selectors: Selectors { kernel_cs, kernel_ds, user_cs, user_ds, tss },
        }
    };
}

pub struct Gdt {
    table: GlobalDescriptorTable,
    pub selectors: Selectors,
}

pub struct Selectors {
    pub kernel_cs: SegmentSelector,
    pub kernel_ds: SegmentSelector,
    pub user_cs: SegmentSelector,
    pub user_ds: SegmentSelector,
    pub tss: SegmentSelector,
}

pub fn init() {
    use x86_64::instructions::segmentation::*;
    use x86_64::instructions::tables::load_tss;

    debug!("gdt: initialising rust gdt");

    GDT.table.load();

    // SAFETY: all of these values are correct.
    unsafe {
        set_cs(GDT.selectors.kernel_cs);
        load_tss(GDT.selectors.tss);

        // Reload selector registers
        load_ss(GDT.selectors.kernel_ds);
        load_ds(GDT.selectors.kernel_ds);
        load_es(GDT.selectors.kernel_ds);
        load_fs(GDT.selectors.kernel_ds);
        load_gs(GDT.selectors.kernel_ds);
    }

    debug!("gdt: initialised");
}
