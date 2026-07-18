use crate::println;
use limine::memmap::{Entry, MEMMAP_USABLE};

pub const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = 1_048_576; // 4 GB
const BITMAP_SIZE: usize = MAX_FRAMES / 8;

static mut BITMAP: [u8; BITMAP_SIZE] = [0xFF; BITMAP_SIZE];
static mut FREE_FRAMES: usize = 0;

pub fn init(memory_map: &[&Entry]) {
    unsafe {
        FREE_FRAMES = 0;

        for entry in memory_map {
            if entry.type_ == MEMMAP_USABLE {
                let base = entry.base as usize;
                let length = entry.length as usize;

                let start_frame = base / FRAME_SIZE;
                let frame_count = length / FRAME_SIZE;

                for i in 0..frame_count {
                    let frame_index = start_frame + i;
                    if frame_index < MAX_FRAMES {
                        set_frame_free(frame_index);
                        FREE_FRAMES += 1;
                    }
                }
            }
        }

        // Безопасное чтение из mutable static (убирает warning)
        let free_frames = core::ptr::addr_of!(FREE_FRAMES).read();
        let free_ram_mb = (free_frames * FRAME_SIZE) / (1024 * 1024);
        println!("[MEM] PMM initialized. Free RAM: {} MB ({} frames)", free_ram_mb, free_frames);
    }
}

pub fn alloc_frame() -> Option<usize> {
    unsafe {
        for byte_idx in 0..BITMAP_SIZE {
            if BITMAP[byte_idx] != 0xFF {
                for bit_idx in 0..8 {
                    let frame_index = byte_idx * 8 + bit_idx;
                    if is_frame_free(frame_index) {
                        set_frame_used(frame_index);
                        FREE_FRAMES -= 1;
                        return Some(frame_index * FRAME_SIZE);
                    }
                }
            }
        }
        None
    }
}

pub fn free_frame(addr: usize) {
    if addr % FRAME_SIZE != 0 {
        println!("[MEM] ERROR: Attempt to free unaligned address!");
        return;
    }
    
    let frame_index = addr / FRAME_SIZE;
    if frame_index >= MAX_FRAMES {
        return;
    }

    unsafe {
        if !is_frame_free(frame_index) {
            set_frame_free(frame_index);
            FREE_FRAMES += 1;
        } else {
            println!("[MEM] WARNING: Double free detected!");
        }
    }
}

#[inline]
unsafe fn set_frame_used(index: usize) {
    BITMAP[index / 8] |= 1 << (index % 8);
}

#[inline]
unsafe fn set_frame_free(index: usize) {
    BITMAP[index / 8] &= !(1 << (index % 8));
}

#[inline]
unsafe fn is_frame_free(index: usize) -> bool {
    (BITMAP[index / 8] & (1 << (index % 8))) == 0
}