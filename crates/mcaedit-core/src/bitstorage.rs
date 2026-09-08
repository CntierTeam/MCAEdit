//! Minecraft `SimpleBitStorage`: tightly packed integers, `values_per_long = 64 / bits`.

#[derive(Clone, Debug)]
pub struct BitStorage {
    bits: u8,
    size: usize,
    data: Vec<i64>,
    mask: u64,
    values_per_long: usize,
}

impl BitStorage {
    pub fn new(bits: u8, size: usize) -> Self {
        assert!((1..=32).contains(&bits), "bits must be 1..=32");
        let values_per_long = 64 / bits as usize;
        let len = size.div_ceil(values_per_long);
        Self {
            bits,
            size,
            data: vec![0; len],
            mask: (1u64 << bits) - 1,
            values_per_long,
        }
    }

    pub fn from_raw(bits: u8, size: usize, data: Vec<i64>) -> Result<Self, String> {
        let mut storage = Self::new(bits, size);
        let expected = storage.data.len();
        if data.len() != expected {
            return Err(format!(
                "bitstorage length mismatch: got {} expected {} (bits={bits}, size={size})",
                data.len(),
                expected
            ));
        }
        storage.data = data;
        Ok(storage)
    }

    pub fn bits(&self) -> u8 {
        self.bits
    }

    pub fn raw(&self) -> &[i64] {
        &self.data
    }

    pub fn into_raw(self) -> Vec<i64> {
        self.data
    }

    pub fn get(&self, index: usize) -> u32 {
        debug_assert!(index < self.size);
        let cell = index / self.values_per_long;
        let offset = (index % self.values_per_long) * self.bits as usize;
        ((self.data[cell] as u64) >> offset & self.mask) as u32
    }

    pub fn set(&mut self, index: usize, value: u32) {
        debug_assert!(index < self.size);
        debug_assert!((value as u64) <= self.mask);
        let cell = index / self.values_per_long;
        let offset = (index % self.values_per_long) * self.bits as usize;
        let clear = !(self.mask << offset);
        let write = (self.data[cell] as u64 & clear) | ((value as u64 & self.mask) << offset);
        self.data[cell] = write as i64;
    }

    pub fn unpack(&self) -> Vec<u32> {
        (0..self.size).map(|i| self.get(i)).collect()
    }

    pub fn pack_values(bits: u8, values: &[u32]) -> Self {
        let mut storage = Self::new(bits, values.len());
        for (i, &v) in values.iter().enumerate() {
            storage.set(i, v);
        }
        storage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_4bit() {
        let values: Vec<u32> = (0..4096).map(|i| (i % 16) as u32).collect();
        let packed = BitStorage::pack_values(4, &values);
        assert_eq!(packed.raw().len(), 256);
        assert_eq!(packed.unpack(), values);
    }
}
