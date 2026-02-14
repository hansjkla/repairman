use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, Arc};


#[derive(Debug)]
pub struct BufferPool {
    buffer_size: usize,
    free_buffers: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl BufferPool {
    pub fn new(buffer_size: usize, initial_capacity: usize) -> BufferPool {
        let mut free_buffers = Vec::with_capacity(initial_capacity);
        for _ in 0..initial_capacity {
            free_buffers.push(vec![0u8; buffer_size]);
        }

        BufferPool {
            buffer_size,
            free_buffers: Arc::new(Mutex::new(free_buffers)),
        }
    }

    pub fn lease(&self) -> BufferLease {
        let buffer = self.free_buffers.as_ref().lock().expect("poisend").pop()
            .unwrap_or_else(|| vec![0u8; self.buffer_size]);


        BufferLease { buffer_pool: self.free_buffers.clone(), buffer: Some(buffer) }
    }

    pub fn get_size(&self) -> usize {
        self.buffer_size
    }
}

#[derive(Debug)]
pub struct BufferLease {
    buffer_pool: Arc<Mutex<Vec<Vec<u8>>>>,
    buffer: Option<Vec<u8>>,
}

impl Drop for BufferLease {
    fn drop(&mut self) {
        if let Some(mut buf) = self.buffer.take() {

            let cap = buf.capacity();
            unsafe { buf.set_len(cap); }

            self.buffer_pool.lock().unwrap().push(buf);
        }
    }
}

impl Deref for BufferLease {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        self.buffer.as_ref().unwrap()
    }
}

impl DerefMut for BufferLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer.as_mut().unwrap()
    }
}

#[test]
fn test() {
    let pool = BufferPool::new(32, 1);
    
    {
        let mut test = pool.lease();

        test[0] = 23;
        dbg!(test.len());
    }

    let test = pool.lease();
    dbg!(test.len());
    let test1 = pool.lease();

    dbg!(test1);
    dbg!(test);
    dbg!(pool);
}