// from https://github.com/rust-lang/rust/commit/eca1cc957fff157575f485ebfd2aaafb33ee98cb

use std::collections::{HashMap as DefaultHashMap, HashSet as DefaultHashSet};
use std::default::Default;
use std::hash::{Hasher, BuildHasherDefault};
use std::ops::BitXor;

use byteorder::{ByteOrder, NativeEndian, LittleEndian};

use uuid::Uuid;

pub type HashMap<K, V> = DefaultHashMap<K, V, BuildHasherDefault<FxHasher>>;
#[allow(dead_code)]
pub type HashSet<K> = DefaultHashSet<K, BuildHasherDefault<FxHasher>>;

pub type UuidHashMap<V> = DefaultHashMap<Uuid, V, BuildHasherDefault<UuidHasher>>;
pub type UuidHashSet = DefaultHashSet<Uuid, BuildHasherDefault<UuidHasher>>;

pub type ClientIdHashMap<V> =
    DefaultHashMap<::socket_addr::Ipv4SocketAddr, V, BuildHasherDefault<UuidHasher>>;
pub type ClientIdSet =
    DefaultHashSet<::socket_addr::Ipv4SocketAddr, BuildHasherDefault<UuidHasher>>;


pub type IdHashMap<K, V> =
    DefaultHashMap<K, V, BuildHasherDefault<IdHasher>>;
pub type IdSet<V> =
    DefaultHashSet<V, BuildHasherDefault<IdHasher>>;


pub struct FxHasher {
    hash: usize
}

const K: usize = 0x517cc1b727220a95;

impl Default for FxHasher {
    #[inline]
    fn default() -> FxHasher {
        FxHasher { hash: 0 }
    }
}


impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, i: usize) {
        self.hash = self.hash.rotate_left(5).bitxor(i).wrapping_mul(K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            let i = *byte;
            self.add_to_hash(i as usize);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(i as usize);
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(i as usize);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(i as usize);
    }

    #[cfg(target_pointer_width = "32")]
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i as usize);
        self.add_to_hash((i >> 32) as usize);
    }

    #[cfg(target_pointer_width = "64")]
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i as usize);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash as u64
    }
}

#[derive(Default)]
pub struct UuidHasher(u64);

impl Hasher for UuidHasher {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        if bytes.len() == 16 {
            //TODO only take the random half
            let data0 = LittleEndian::read_u64(&bytes[..8]);
            let data1 = LittleEndian::read_u64(&bytes[8..16]);
            // currently rust xor's the len pf a slice into the hash
            // since all our slices are the same len there is no need
            // should we add ^ 16?
            self.0 = data0 ^ data1;
            return
        }

        for &b in bytes {
            self.0 = self.0.rotate_left(1) ^ b as u64;
        }
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) {
        match bytes.len() {
            //TODO slice pattern
            0 => (),
            1 => self.write_u8(bytes[0]),
            2 => self.write_u16(NativeEndian::read_u16(bytes)),
            4 => self.write_u32(NativeEndian::read_u32(bytes)),
            8 => self.write_u64(NativeEndian::read_u64(bytes)),
            16 => {
                let (b0, b1) = bytes.split_at(8);
                self.0 = NativeEndian::read_u64(b0) ^ NativeEndian::read_u64(b1)
            },
            //TODO should we xor the len?
            //     this shouldn't really be used for slices...
            _ => for &b in bytes {
                self.0 = self.0.rotate_left(1) ^ b as u64;
            },
        }
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    fn write_u8(&mut self, i: u8) {
        self.0 = i as _
    }
    fn write_u16(&mut self, i: u16) {
        self.0 = i as _
    }
    fn write_u32(&mut self, i: u32) {
        self.0 = i as _
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = i as _
    }

    fn write_usize(&mut self, i: usize) {
        self.0 = i as _
    }

    fn write_i8(&mut self, i: i8) {
        self.0 = i as _
    }
    fn write_i16(&mut self, i: i16) {
        self.0 = i as _
    }
    fn write_i32(&mut self, i: i32) {
        self.0 = i as _
    }

    fn write_i64(&mut self, i: i64) {
        self.0 = i as _
    }

    fn write_isize(&mut self, i: isize) {
        self.0 = i as _
    }
}
