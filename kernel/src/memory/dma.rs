use crate::memory::{paging, pmm};
use core::ptr::NonNull;

/// Physically contiguous, page-aligned memory for device DMA.
pub struct DmaBuffer {
    virtual_address: NonNull<u8>,
    physical_address: usize,
    requested_len: usize,
    page_count: usize,
}

impl DmaBuffer {
    /// Allocate zeroed DMA memory through the kernel HHDM.
    pub fn allocate(size: usize) -> Option<Self> {
        if size == 0 {
            return None;
        }

        let page_count = size.checked_add(pmm::FRAME_SIZE - 1)? / pmm::FRAME_SIZE;
        let physical_address = pmm::alloc_contiguous(page_count)?;
        let hhdm = paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed) as usize;
        let virtual_address = match hhdm.checked_add(physical_address) {
            Some(address) => match NonNull::new(address as *mut u8) {
                Some(pointer) => pointer,
                None => {
                    pmm::free_contiguous(physical_address, page_count);
                    return None;
                }
            },
            None => {
                pmm::free_contiguous(physical_address, page_count);
                return None;
            }
        };

        let allocated_len = page_count.checked_mul(pmm::FRAME_SIZE)?;
        unsafe { core::ptr::write_bytes(virtual_address.as_ptr(), 0, allocated_len); }

        Some(Self {
            virtual_address,
            physical_address,
            requested_len: size,
            page_count,
        })
    }

    pub fn physical_address(&self) -> usize {
        self.physical_address
    }

    pub fn len(&self) -> usize {
        self.requested_len
    }

    pub fn is_empty(&self) -> bool {
        self.requested_len == 0
    }

    pub fn allocated_len(&self) -> usize {
        self.page_count * pmm::FRAME_SIZE
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.virtual_address.as_ptr()
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.virtual_address.as_ptr()
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.virtual_address.as_ptr(), self.requested_len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.virtual_address.as_ptr(), self.requested_len) }
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        pmm::free_contiguous(self.physical_address, self.page_count);
    }
}