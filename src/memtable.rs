use std::collections::BTreeMap;

#[derive(Debug, PartialEq)]
pub enum ValueState<'a> {
    Value(&'a [u8]),
    Tombstone,
    NotFound,
}

pub struct MemTable {
    map: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    approximate_size: usize,
}

impl MemTable {
    /// Creates a new , empty memTable.
    pub fn new() -> Self{
        Self {
            map: BTreeMap::new(),
            approximate_size: 0,
        }
    }

    ///Inserts or updates a key-value pair
    pub fn put(&mut self, key: &[u8], value: &[u8]) {
        if let Some(old_entry) = self.map.get(key) {
            if let Some(old_val) = old_entry {
                self.approximate_size -= old_val.len();
            }
        } else {
            self.approximate_size += key.len();
        }

        self.approximate_size += value.len();
        self.map.insert(key.to_vec(), Some(value.to_vec()));

    }

    pub fn delete(&mut self, key: &[u8]) {
        if let Some(old_entry) = self.map.get(key) {
            if let Some(old_val) = old_entry {
                self.approximate_size -= old_val.len();
            }
        } else {
            self.approximate_size += key.len();
        }

        self.map.insert(key.to_vec(), None);
    }

    pub fn get(&self, key: &[u8]) ->ValueState<'_> {
        match self.map.get(key) {
            Some(Some(val)) => ValueState::Value(val.as_slice()),
            Some(None) => ValueState::Tombstone,
            None => ValueState::NotFound,
        }
    }

    pub fn approximate_size(&self) ->usize {
        self.approximate_size
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) ->bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8], Option<&[u8]>)>+ '_ {
        self.map.iter().map(|(k,v)| (k.as_slice(), v.as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memtable_basic() {
        let mut memtable = MemTable::new();
        assert!(memtable.is_empty());

        //1.put key
        memtable.put(b"key1", b"val1");
        assert_eq!(memtable.len(), 1);
        assert_eq!(memtable.get(b"key1"), ValueState::Value(b"val1"));
        assert_eq!(memtable.approximate_size(), 4 + 4);

        // 2. Overwrite with larger value
        memtable.put(b"key1", b"val_longer");
        assert_eq!(memtable.len(), 1);
        assert_eq!(memtable.get(b"key1"), ValueState::Value(b"val_longer"));
        assert_eq!(memtable.approximate_size(), 4 + 10);

        // 3. Delete key (Tombstone)
        memtable.delete(b"key1");
        assert_eq!(memtable.len(), 1);
        assert_eq!(memtable.get(b"key1"), ValueState::Tombstone);
        assert_eq!(memtable.approximate_size(), 4); // only key is kept

        // 4. Missing key
        assert_eq!(memtable.get(b"missing"), ValueState::NotFound);
    }

    #[test]
    fn test_memtable_sorted_itteration(){
        let mut memtable = MemTable::new();

        memtable.put(b"banana", b"2");
        memtable.put(b"apple", b"1");
        memtable.put(b"cherry", b"3");

        let keys: Vec<&[u8]> = memtable.iter().map(|(k, _)| k).collect();

        assert_eq!(keys, vec![b"apple".as_slice(), b"banana".as_slice(), b"cherry".as_slice()]);
    }
}