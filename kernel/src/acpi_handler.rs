use acpi::{self, AcpiHandler, AcpiError, AcpiTables};
use crate::memory::physical_mapping::{self, PhysicalMapping};

pub fn acpi_init() -> Result<AcpiTables<WolffiaAcpiHandler>, AcpiError> {
    info!("acpi: initializing");
    let handler = WolffiaAcpiHandler;
    // We're BIOS. We'd have crashed by now if we weren't.
    let search_result = unsafe { AcpiTables::search_for_rsdp_bios(handler) };

    match search_result {
        Ok(tables) => {
            info!("acpi: init successful");
            Ok(tables)
        }
        Err(e) => {
            error!("acpi: init unsuccessful {:?}", e);
            Err(e)
        }
    }
}

#[derive(Clone)]
pub struct WolffiaAcpiHandler;

impl AcpiHandler for WolffiaAcpiHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> acpi::PhysicalMapping<Self, T> {
        // Map immutable region
        let region: PhysicalMapping<T> = physical_mapping::map_physical_region(
            physical_address as u64,
            size as u64,
            false
        );

        region.into_acpi(self.clone())
    }

    fn unmap_physical_region<T>(&self, region: &acpi::PhysicalMapping<Self, T>) {
        let obj_addr = region.virtual_start.as_ptr() as *mut T as usize;

        // Clear lower page offset bits
        let page_begin = obj_addr & !0xFFF;

        unsafe {
            crate::HEAP.dealloc_specific(
                page_begin as *mut u8,
                region.mapped_length as u64 / 4096,
            );
        }
    }
}
