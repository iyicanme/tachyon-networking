use std::collections::VecDeque;

pub const BYTE_BUFFER_SIZE_DEFAULT: usize = 1240;
const POOL_SIZE_DEFAULT: usize = 512;

pub struct ByteBuffer {
    data: Vec<u8>,
    pub length: usize,
    pub pooled: bool,
    pub version: u64,
}

impl ByteBuffer {
    #[must_use]
    pub fn create(length: usize) -> Self {
        Self {
            data: vec![0; length],
            length,
            pooled: false,
            version: 0,
        }
    }

    #[must_use]
    pub fn get(&self) -> &[u8] {
        &self.data
    }

    pub fn get_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

impl<Idx> std::ops::Index<Idx> for ByteBuffer
where
    Idx: std::slice::SliceIndex<[u8]>,
{
    type Output = Idx::Output;

    fn index(&self, index: Idx) -> &Self::Output {
        &self.data[index]
    }
}

impl<Idx> std::ops::IndexMut<Idx> for ByteBuffer
where
    Idx: std::slice::SliceIndex<[u8]>,
{
    fn index_mut(&mut self, index: Idx) -> &mut Self::Output {
        &mut self.data[index]
    }
}

pub struct ByteBufferPool {
    pub buffer_size: usize,
    pool_size: usize,
    buffers: VecDeque<ByteBuffer>,
    count: usize,
}

impl ByteBufferPool {
    #[must_use]
    pub fn create(buffer_size: usize, max_buffers: usize) -> Self {
        Self {
            buffer_size,
            pool_size: max_buffers,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn return_buffer(&mut self, mut byte_buffer: ByteBuffer) -> bool {
        let space_available =
            (byte_buffer.length <= self.buffer_size) && (self.count < self.pool_size);

        if space_available {
            byte_buffer.version += 1;
            self.buffers.push_back(byte_buffer);
            self.count += 1;
        }

        space_available
    }

    pub fn get_buffer(&mut self, length: usize) -> ByteBuffer {
        if length > self.buffer_size {
            return ByteBuffer {
                data: vec![0; length],
                length,
                pooled: false,
                version: 0,
            };
        }

        let Some(mut pooled) = self.buffers.pop_front() else {
            return ByteBuffer {
                data: vec![0; self.buffer_size],
                length,
                pooled: true,
                version: 0,
            };
        };

        self.count -= 1;
        pooled.length = length;
        pooled
    }
}

impl Default for ByteBufferPool {
    fn default() -> Self {
        Self {
            buffer_size: BYTE_BUFFER_SIZE_DEFAULT,
            pool_size: POOL_SIZE_DEFAULT,
            buffers: VecDeque::new(),
            count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::byte_buffer_pool::{
        ByteBuffer, ByteBufferPool, BYTE_BUFFER_SIZE_DEFAULT, POOL_SIZE_DEFAULT,
    };

    #[test]
    fn get_return_within_limits() {
        let mut pool = ByteBufferPool::default();

        let buffer = pool.get_buffer(BYTE_BUFFER_SIZE_DEFAULT);
        assert!(pool.return_buffer(buffer));
        assert_eq!(1, pool.len());
    }

    #[test]
    fn get_allocates_over_max() {
        let mut pool = ByteBufferPool::default();

        let buffer = pool.get_buffer(BYTE_BUFFER_SIZE_DEFAULT);
        pool.return_buffer(buffer);
        pool.get_buffer(BYTE_BUFFER_SIZE_DEFAULT + 1);
        assert_eq!(1, pool.len());
    }

    #[test]
    fn will_not_return_over_max_buffer_size() {
        let mut pool = ByteBufferPool::default();

        let buffer = pool.get_buffer(BYTE_BUFFER_SIZE_DEFAULT + 1);
        assert!(!pool.return_buffer(buffer));
        assert_eq!(0, pool.len());
    }

    #[test]
    fn will_not_return_if_full() {
        let mut pool = ByteBufferPool::default();

        for _ in 0..POOL_SIZE_DEFAULT {
            let buffer = ByteBuffer::create(1024);
            assert!(pool.return_buffer(buffer));
        }

        let buffer = ByteBuffer::create(1024);
        assert!(!pool.return_buffer(buffer));
    }
}
