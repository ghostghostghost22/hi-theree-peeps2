//! Guest physical memory management.
//!
//! The memory object owns page-aligned host allocations and exposes only
//! checked guest-physical accesses. Hypervisor backends may borrow the region
//! descriptors to register those allocations with the host.

use std::{
    alloc::{alloc_zeroed, dealloc, Layout},
    cmp::min,
    fmt,
    ptr::NonNull,
};

pub const PAGE_SIZE: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuestAddress(pub u64);

impl fmt::Display for GuestAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{:x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    InvalidRegion(String),
    AllocationFailed { size: usize },
    AddressOutOfBounds { address: GuestAddress, length: usize },
    ArithmeticOverflow,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegion(message) => {
                write!(formatter, "invalid guest-memory region: {message}")
            }
            Self::AllocationFailed { size } => {
                write!(formatter, "unable to allocate {size} bytes of guest memory")
            }
            Self::AddressOutOfBounds { address, length } => {
                write!(
                    formatter,
                    "guest-memory access at {address} with length {length} is out of bounds"
                )
            }
            Self::ArithmeticOverflow => {
                formatter.write_str("guest-memory address arithmetic overflowed")
            }
        }
    }
}

impl std::error::Error for MemoryError {}

struct AlignedAllocation {
    pointer: NonNull<u8>,
    length: usize,
    layout: Layout,
}

// The allocation is exclusively owned. Moving it to a vCPU thread does not
// create aliases; callers still need synchronization before sharing guest RAM.
unsafe impl Send for AlignedAllocation {}

impl AlignedAllocation {
    fn new(length: usize) -> Result<Self, MemoryError> {
        let layout = Layout::from_size_align(length, PAGE_SIZE as usize)
            .map_err(|_| MemoryError::InvalidRegion("allocation layout is invalid".to_owned()))?;
        // `length > 0` is enforced by region validation, so a null pointer is
        // always an allocation failure rather than a valid dangling pointer.
        let raw = unsafe { alloc_zeroed(layout) };
        let pointer = NonNull::new(raw).ok_or(MemoryError::AllocationFailed { size: length })?;
        Ok(Self {
            pointer,
            length,
            layout,
        })
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.length) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.length) }
    }

    fn address(&self) -> usize {
        self.pointer.as_ptr() as usize
    }
}

impl Drop for AlignedAllocation {
    fn drop(&mut self) {
        unsafe { dealloc(self.pointer.as_ptr(), self.layout) };
    }
}

struct MemoryRegion {
    guest_base: GuestAddress,
    allocation: AlignedAllocation,
}

/// A borrowed view of one registered guest-memory region.
pub struct GuestMemoryRegion<'a> {
    region: &'a MemoryRegion,
}

impl GuestMemoryRegion<'_> {
    pub const fn guest_base(&self) -> GuestAddress {
        self.region.guest_base
    }

    pub const fn len(&self) -> u64 {
        self.region.allocation.length as u64
    }

    /// Host virtual address used by a hypervisor memory-registration API.
    ///
    /// The address is valid only while the originating [`GuestMemory`] remains
    /// alive. The memory object does not expose operations that resize or
    /// reorder regions.
    pub fn host_address(&self) -> usize {
        self.region.allocation.address()
    }

    pub const fn end_address(&self) -> GuestAddress {
        GuestAddress(self.guest_base.0 + self.len())
    }
}

/// Page-aligned, checked guest physical memory.
pub struct GuestMemory {
    regions: Vec<MemoryRegion>,
}

impl GuestMemory {
    pub fn new(size: u64) -> Result<Self, MemoryError> {
        Self::with_regions([(GuestAddress(0), size)])
    }

    pub fn with_regions<I>(regions: I) -> Result<Self, MemoryError>
    where
        I: IntoIterator<Item = (GuestAddress, u64)>,
    {
        let mut specifications: Vec<_> = regions.into_iter().collect();
        if specifications.is_empty() {
            return Err(MemoryError::InvalidRegion(
                "at least one region is required".to_owned(),
            ));
        }
        specifications.sort_by_key(|(base, _)| *base);

        let mut allocated = Vec::with_capacity(specifications.len());
        let mut previous_end = 0u64;
        for (index, (guest_base, size)) in specifications.into_iter().enumerate() {
            if size == 0 {
                return Err(MemoryError::InvalidRegion(format!(
                    "region {index} has zero length"
                )));
            }
            if guest_base.0 % PAGE_SIZE != 0 || size % PAGE_SIZE != 0 {
                return Err(MemoryError::InvalidRegion(format!(
                    "region {index} must be aligned to {PAGE_SIZE} bytes"
                )));
            }
            let end = guest_base
                .0
                .checked_add(size)
                .ok_or(MemoryError::ArithmeticOverflow)?;
            if guest_base.0 < previous_end {
                return Err(MemoryError::InvalidRegion(format!(
                    "region {index} overlaps a previous region"
                )));
            }
            let length = usize::try_from(size).map_err(|_| {
                MemoryError::InvalidRegion(format!("region {index} is too large for this host"))
            })?;
            allocated.push(MemoryRegion {
                guest_base,
                allocation: AlignedAllocation::new(length)?,
            });
            previous_end = end;
        }
        Ok(Self { regions: allocated })
    }

    pub fn regions(&self) -> impl Iterator<Item = GuestMemoryRegion<'_>> {
        self.regions.iter().map(|region| GuestMemoryRegion { region })
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn size(&self) -> u64 {
        self.regions
            .iter()
            .map(|region| region.allocation.length as u64)
            .sum()
    }

    pub fn max_address(&self) -> GuestAddress {
        self.regions()
            .map(|region| region.end_address())
            .max()
            .unwrap_or(GuestAddress(0))
    }

    pub fn read(&self, address: GuestAddress, destination: &mut [u8]) -> Result<(), MemoryError> {
        if destination.is_empty() {
            return Ok(());
        }
        let length = destination.len();
        self.validate_range(address, length)?;
        let mut current = address.0;
        let mut offset = 0usize;
        while offset < length {
            let region = self
                .find_region(GuestAddress(current))
                .ok_or(MemoryError::AddressOutOfBounds { address, length })?;
            let region_offset = usize::try_from(current - region.guest_base.0)
                .map_err(|_| MemoryError::ArithmeticOverflow)?;
            let count = min(region.allocation.length - region_offset, length - offset);
            destination[offset..offset + count]
                .copy_from_slice(&region.allocation.as_slice()[region_offset..region_offset + count]);
            current = current
                .checked_add(u64::try_from(count).map_err(|_| MemoryError::ArithmeticOverflow)?)
                .ok_or(MemoryError::ArithmeticOverflow)?;
            offset += count;
        }
        Ok(())
    }

    pub fn write(&mut self, address: GuestAddress, source: &[u8]) -> Result<(), MemoryError> {
        if source.is_empty() {
            return Ok(());
        }
        let length = source.len();
        self.validate_range(address, length)?;
        let mut current = address.0;
        let mut offset = 0usize;
        while offset < length {
            let region = self
                .regions
                .iter_mut()
                .find(|region| {
                    current >= region.guest_base.0
                        && current < region.guest_base.0 + region.allocation.length as u64
                })
                .ok_or(MemoryError::AddressOutOfBounds { address, length })?;
            let region_offset = usize::try_from(current - region.guest_base.0)
                .map_err(|_| MemoryError::ArithmeticOverflow)?;
            let count = min(region.allocation.length - region_offset, length - offset);
            region.allocation.as_mut_slice()[region_offset..region_offset + count]
                .copy_from_slice(&source[offset..offset + count]);
            current = current
                .checked_add(u64::try_from(count).map_err(|_| MemoryError::ArithmeticOverflow)?)
                .ok_or(MemoryError::ArithmeticOverflow)?;
            offset += count;
        }
        Ok(())
    }

    pub fn read_u8(&self, address: GuestAddress) -> Result<u8, MemoryError> {
        let mut value = [0u8; 1];
        self.read(address, &mut value)?;
        Ok(value[0])
    }

    pub fn write_u8(&mut self, address: GuestAddress, value: u8) -> Result<(), MemoryError> {
        self.write(address, &[value])
    }

    pub fn host_address(&self, address: GuestAddress) -> Result<usize, MemoryError> {
        let region = self
            .find_region(address)
            .ok_or(MemoryError::AddressOutOfBounds { address, length: 1 })?;
        let offset = usize::try_from(address.0 - region.guest_base.0)
            .map_err(|_| MemoryError::ArithmeticOverflow)?;
        Ok(region.allocation.address() + offset)
    }

    fn validate_range(&self, address: GuestAddress, length: usize) -> Result<(), MemoryError> {
        let length_u64 = u64::try_from(length).map_err(|_| MemoryError::ArithmeticOverflow)?;
        address
            .0
            .checked_add(length_u64)
            .ok_or(MemoryError::ArithmeticOverflow)?;

        let mut current = address.0;
        let mut remaining = length;
        while remaining > 0 {
            let region = self
                .find_region(GuestAddress(current))
                .ok_or(MemoryError::AddressOutOfBounds { address, length })?;
            let offset = usize::try_from(current - region.guest_base.0)
                .map_err(|_| MemoryError::ArithmeticOverflow)?;
            let count = min(region.allocation.length - offset, remaining);
            current = current
                .checked_add(u64::try_from(count).map_err(|_| MemoryError::ArithmeticOverflow)?)
                .ok_or(MemoryError::ArithmeticOverflow)?;
            remaining -= count;
        }
        Ok(())
    }

    fn find_region(&self, address: GuestAddress) -> Option<&MemoryRegion> {
        self.regions.iter().find(|region| {
            address.0 >= region.guest_base.0
                && address.0 < region.guest_base.0 + region.allocation.length as u64
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_memory() {
        let mut memory = GuestMemory::with_regions([
            (GuestAddress(0), PAGE_SIZE),
            (GuestAddress(PAGE_SIZE), PAGE_SIZE),
        ])
        .expect("regions are valid");
        memory
            .write(GuestAddress(PAGE_SIZE - 2), b"hello")
            .expect("write can cross adjacent regions");
        let mut result = [0u8; 5];
        memory
            .read(GuestAddress(PAGE_SIZE - 2), &mut result)
            .expect("read can cross adjacent regions");
        assert_eq!(&result, b"hello");
    }

    #[test]
    fn rejects_unmapped_access() {
        let memory = GuestMemory::new(PAGE_SIZE).expect("memory is valid");
        let mut value = [0u8; 1];
        assert!(matches!(
            memory.read(GuestAddress(PAGE_SIZE), &mut value),
            Err(MemoryError::AddressOutOfBounds { .. })
        ));
    }

    #[test]
    fn gap_failures_do_not_partially_write() {
        let mut memory = GuestMemory::with_regions([
            (GuestAddress(0), PAGE_SIZE),
            (GuestAddress(PAGE_SIZE * 2), PAGE_SIZE),
        ])
        .expect("regions are valid");
        let source = [0xa5; 2];
        assert!(memory.write(GuestAddress(PAGE_SIZE - 1), &source).is_err());
        assert_eq!(memory.read_u8(GuestAddress(PAGE_SIZE - 1)).expect("byte is mapped"), 0);
    }

    #[test]
    fn rejects_overlapping_regions() {
        let result = GuestMemory::with_regions([
            (GuestAddress(0), PAGE_SIZE),
            (GuestAddress(PAGE_SIZE / 2), PAGE_SIZE),
        ]);
        assert!(matches!(result, Err(MemoryError::InvalidRegion(_))));
    }

    #[test]
    fn regions_are_page_aligned_for_hypervisor_registration() {
        let memory = GuestMemory::new(PAGE_SIZE * 2).expect("memory is valid");
        let region = memory.regions().next().expect("one region");
        assert_eq!(region.host_address() % PAGE_SIZE as usize, 0);
    }
}
