//! Host-only immutable byte snapshot and lifetime pins, NOT guest TCP support.
#![allow(dead_code)]
use std::sync::{Arc, Mutex};

const MAX_BYTES: usize = 1024 * 1024;

struct Metadata {
    pins: usize,
    retiring: bool,
}

struct Inner {
    bytes: Box<[u8]>,
    control: Mutex<Metadata>,
}

pub(crate) struct FrozenTcpSlab(Arc<Inner>);

pub(crate) struct Pin {
    inner: Arc<Inner>,
    live: bool,
}

impl FrozenTcpSlab {
    pub(crate) fn new(source: &[u8]) -> Result<Self, &'static str> {
        if source.len() > MAX_BYTES {
            return Err("TCP slab exceeds one MiB");
        }
        Ok(Self(Arc::new(Inner {
            bytes: source.to_vec().into_boxed_slice(),
            control: Mutex::new(Metadata {
                pins: 0,
                retiring: false,
            }),
        })))
    }

    pub(crate) fn size(&self) -> usize {
        self.0.bytes.len()
    }

    pub(crate) fn pin(&self) -> Option<Pin> {
        let mut m = self.0.control.lock().ok()?;
        if m.retiring || m.pins == usize::MAX {
            return None;
        }
        m.pins += 1;
        Some(Pin {
            inner: Arc::clone(&self.0),
            live: true,
        })
    }

    pub(crate) fn read_byte(&self, pin: &Pin, index: usize) -> Option<u8> {
        let _guard = self.0.control.lock().ok()?;
        if !pin.live || !Arc::ptr_eq(&pin.inner, &self.0) {
            return None;
        }
        self.0.bytes.get(index).copied()
    }

    pub(crate) fn release_pin(&self, pin: &mut Pin) -> bool {
        if !Arc::ptr_eq(&pin.inner, &self.0) {
            return false;
        }
        pin.release()
    }

    pub(crate) fn retire(&self) {
        self.0
            .control
            .lock()
            .expect("slab metadata poisoned")
            .retiring = true;
    }

    pub(crate) fn freed(&self) -> bool {
        let m = self.0.control.lock().expect("slab metadata poisoned");
        m.retiring && m.pins == 0
    }
}

impl Pin {
    fn release(&mut self) -> bool {
        if !self.live {
            return false;
        }
        let mut m = self.inner.control.lock().expect("slab metadata poisoned");
        self.live = false;
        m.pins -= 1;
        true
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_bytes_clone_at_publication_and_pins_retire() {
        let mut source = [0, 1, 127, 128, 255];
        let slab = FrozenTcpSlab::new(&source).unwrap();
        source.fill(0);
        let mut first = slab.pin().unwrap();
        let mut second = slab.pin().unwrap();
        assert_eq!(slab.size(), 5);
        assert_eq!(slab.read_byte(&first, 3), Some(128));
        assert_eq!(slab.read_byte(&first, 4), Some(255));
        assert_eq!(slab.read_byte(&first, 5), None);
        slab.retire();
        assert!(slab.pin().is_none());
        assert!(slab.release_pin(&mut first));
        assert!(!slab.release_pin(&mut first));
        assert!(!slab.freed());
        assert_eq!(slab.read_byte(&second, 3), Some(128));
        assert!(slab.release_pin(&mut second));
        assert!(slab.freed());
        assert_eq!(slab.read_byte(&second, 3), None);
    }

    #[test]
    fn pins_cannot_cross_slab_boundaries() {
        let a = FrozenTcpSlab::new(&[42]).unwrap();
        let b = FrozenTcpSlab::new(&[42]).unwrap();
        let mut pin = a.pin().unwrap();
        assert_eq!(b.read_byte(&pin, 0), None);
        assert!(!b.release_pin(&mut pin));
        assert_eq!(a.read_byte(&pin, 0), Some(42));
        assert!(a.release_pin(&mut pin));
    }

    #[test]
    fn pins_keep_storage_alive_after_owner_drop() {
        let pin = {
            let slab = FrozenTcpSlab::new(&[17]).unwrap();
            slab.pin().unwrap()
        };
        // The pin holds a strong Arc, and Drop releases its metadata safely.
        assert_eq!(pin.inner.bytes[0], 17);
        drop(pin);
    }

    #[test]
    fn empty_and_oversize_snapshots() {
        let empty = FrozenTcpSlab::new(&[]).unwrap();
        let pin = empty.pin().unwrap();
        assert_eq!(empty.read_byte(&pin, 0), None);
        assert!(FrozenTcpSlab::new(&vec![0; MAX_BYTES + 1]).is_err());
    }
}
