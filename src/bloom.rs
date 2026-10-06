use crc32fast::Hasher;

pub struct BloomFilter{
    bitmap: Vec<u8>,
    k: u8, //no of hash iterations (tipically 7 for 10 bits/keys)
}

impl BloomFilter {

    //bloom fileter size for given list of keys with bits = 10 (1% false positive rate)
    pub fn build_from_keys(keys: &[&[u8]], bits_per_key: usize) -> Self {
        if keys.is_empty() {
            return Self { bitmap: Vec::new(), k:0};
        }

        let num_bits = keys.len()*bits_per_key;
        let num_bytes = (num_bits + 7) / 8;
        let total_bits = num_bytes * 8;

        let k = ((bits_per_key as f64 * 0.693).round() as u8).clamp(1, 30);
        let mut bitmap = vec![0u8; num_bytes];

        for key in keys {
            let (h1, h2) = Self::hash_key(key);

            for i in 0..k {

                let bit_idx = (h1.wrapping_add((i as u32).wrapping_mul(h2))) as usize % total_bits;
                let byte_idx = bit_idx/8;
                let bit_pos = bit_idx % 8;
                bitmap[byte_idx] |= 1 << bit_pos;
            }
        }

        Self { bitmap, k}

    }

    pub fn may_contain(&self, key: &[u8]) ->bool {
        if self.bitmap.is_empty() {
            return true;
        }

        let total_bits = self.bitmap.len() * 8;
        let (h1, h2) = Self::hash_key(key);

        for i in 0..self.k {
            let bit_idx = (h1.wrapping_add((i as u32).wrapping_mul(h2))) as usize % total_bits;
            let byte_idx = bit_idx/8;
            let bit_pos = bit_idx % 8;

            if(self.bitmap[byte_idx] & (1 << bit_pos)) == 0 {
                return false;   // forund 0 bit deffinitely DNE
            }
        }

        true
    }

    fn hash_key(key: &[u8]) -> (u32, u32) {
        let mut hasher1 = Hasher::new();
        hasher1.update(key);
        let h1 = hasher1.finalize();


        // rotate bits and mix to create independent second hash
        let h2 = h1.rotate_left(15) ^ 0x5a5a5a5a;
        (h1, h2)
    }

    pub fn bitmap(&self) -> &[u8] {
        &self.bitmap
    }

    pub fn k(&self) ->u8 {
        self.k
    }

    pub fn from_raw(bitmap: Vec<u8>, k: u8) ->Self {
        Self { bitmap, k }
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_filter() {
        let keys = vec![
            b"apple".as_slice(),
            b"banana".as_slice(),
            b"cherry".as_slice(),
            b"date".as_slice(),
            b"elderberry".as_slice(),
        ];

        // 10 bits per key
        let filter = BloomFilter::build_from_keys(&keys, 10);

        // 1. All inserted keys MUST return true (Zero false negatives!)
        for key in &keys {
            assert!(filter.may_contain(key));
        }

        // 2. Non-existent keys should mostly return false
        assert!(!filter.may_contain(b"watermelon"));
        assert!(!filter.may_contain(b"pineapple"));
    }
}
