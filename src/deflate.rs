//! DEFLATE compression and decompression of streams

use std::io::prelude::*;
use std::io;
use std::mem;

#[cfg(feature = "tokio")]
use futures::Poll;
#[cfg(feature = "tokio")]
use tokio_io::{AsyncRead, AsyncWrite};

use bufreader::BufReader;
use zio;
use {Compress, Decompress};

/// A DEFLATE encoder, or compressor.
///
/// This structure implements a [`Write`] interface and takes a stream of
/// uncompressed data, writing the compressed data to the wrapped writer.
///
/// [`Write`]: https://doc.rust-lang.org/std/io/trait.Write.html
#[derive(Debug)]
pub struct EncoderWriter<W: Write> {
    inner: zio::Writer<W, Compress>,
}

/// A DEFLATE encoder, or compressor.
///
/// This structure implements a [`Read`] interface and will read uncompressed
/// data from an underlying stream and emit a stream of compressed data.
///
/// [`Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
#[derive(Debug)]
pub struct EncoderReader<R> {
    inner: EncoderReaderBuf<BufReader<R>>,
}

/// A DEFLATE encoder, or compressor.
///
/// This structure implements a [`BufRead`] interface and will read uncompressed
/// data from an underlying stream and emit a stream of compressed data.
///
/// [`BufRead`]: https://doc.rust-lang.org/std/io/trait.BufRead.html
#[derive(Debug)]
pub struct EncoderReaderBuf<R> {
    obj: R,
    data: Compress,
}

/// A DEFLATE decoder, or decompressor.
///
/// This structure implements a [`Read`] interface and takes a stream of
/// compressed data as input, providing the decompressed data when read from.
///
/// [`Read`]: https://doc.rust-lang.org/std/io/trait.Read.html
#[derive(Debug)]
pub struct DecoderReader<R> {
    inner: DecoderReaderBuf<BufReader<R>>,
}

/// A DEFLATE decoder, or decompressor.
///
/// This structure implements a [`BufRead`] interface and takes a stream of
/// compressed data as input, providing the decompressed data when read from.
///
/// [`BufRead`]: https://doc.rust-lang.org/std/io/trait.BufRead.html
#[derive(Debug)]
pub struct DecoderReaderBuf<R> {
    obj: R,
    data: Decompress,
}

/// A DEFLATE decoder, or decompressor.
///
/// This structure implements a [`Write`] and will emit a stream of decompressed
/// data when fed a stream of compressed data.
///
/// [`Write`]: https://doc.rust-lang.org/std/io/trait.Read.html
#[derive(Debug)]
pub struct DecoderWriter<W: Write> {
    inner: zio::Writer<W, Decompress>,
}

impl<W: Write> EncoderWriter<W> {
    /// Creates a new encoder which will write compressed data to the stream
    /// given at the given compression level.
    ///
    /// When this encoder is dropped or unwrapped the final pieces of data will
    /// be flushed.
    pub fn new(w: W, level: ::Compression) -> EncoderWriter<W> {
        EncoderWriter {
            inner: zio::Writer::new(w, Compress::new(level, false)),
        }
    }

    /// Acquires a reference to the underlying writer.
    pub fn get_ref(&self) -> &W {
        self.inner.get_ref()
    }

    /// Acquires a mutable reference to the underlying writer.
    ///
    /// Note that mutating the output/input state of the stream may corrupt this
    /// object, so care must be taken when using this method.
    pub fn get_mut(&mut self) -> &mut W {
        self.inner.get_mut()
    }

    /// Resets the state of this encoder entirely, swapping out the output
    /// stream for another.
    ///
    /// This function will finish encoding the current stream into the current
    /// output stream before swapping out the two output streams. If the stream
    /// cannot be finished an error is returned.
    ///
    /// After the current stream has been finished, this will reset the internal
    /// state of this encoder and replace the output stream with the one
    /// provided, returning the previous output stream. Future data written to
    /// this encoder will be the compressed into the stream `w` provided.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to complete this stream, and any I/O
    /// errors which occur will be returned from this function.
    pub fn reset(&mut self, w: W) -> io::Result<W> {
        try!(self.inner.finish());
        self.inner.data.reset();
        Ok(self.inner.replace(w))
    }

    /// Attempt to finish this output stream, writing out final chunks of data.
    ///
    /// Note that this function can only be used once data has finished being
    /// written to the output stream. After this function is called then further
    /// calls to `write` may result in a panic.
    ///
    /// # Panics
    ///
    /// Attempts to write data to this stream may result in a panic after this
    /// function is called.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to complete this stream, and any I/O
    /// errors which occur will be returned from this function.
    pub fn try_finish(&mut self) -> io::Result<()> {
        self.inner.finish()
    }

    /// Consumes this encoder, flushing the output stream.
    ///
    /// This will flush the underlying data stream, close off the compressed
    /// stream and, if successful, return the contained writer.
    ///
    /// Note that this function may not be suitable to call in a situation where
    /// the underlying stream is an asynchronous I/O stream. To finish a stream
    /// the `try_finish` (or `shutdown`) method should be used instead. To
    /// re-acquire ownership of a stream it is safe to call this method after
    /// `try_finish` or `shutdown` has returned `Ok`.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to complete this stream, and any I/O
    /// errors which occur will be returned from this function.
    pub fn finish(mut self) -> io::Result<W> {
        try!(self.inner.finish());
        Ok(self.inner.take_inner())
    }

    /// Consumes this encoder, flushing the output stream.
    ///
    /// This will flush the underlying data stream and then return the contained
    /// writer if the flush succeeded.
    /// The compressed stream will not closed but only flushed. This
    /// means that obtained byte array can by extended by another deflated
    /// stream. To close the stream add the two bytes 0x3 and 0x0.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to complete this stream, and any I/O
    /// errors which occur will be returned from this function.
    pub fn flush_finish(mut self) -> io::Result<W> {
        try!(self.inner.flush());
        Ok(self.inner.take_inner())
    }

    /// Returns the number of bytes that have been written to this compresor.
    ///
    /// Note that not all bytes written to this object may be accounted for,
    /// there may still be some active buffering.
    pub fn total_in(&self) -> u64 {
        self.inner.data.total_in()
    }

    /// Returns the number of bytes that the compressor has produced.
    ///
    /// Note that not all bytes may have been written yet, some may still be
    /// buffered.
    pub fn total_out(&self) -> u64 {
        self.inner.data.total_out()
    }
}

impl<W: Write> Write for EncoderWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(feature = "tokio")]
impl<W: AsyncWrite> AsyncWrite for EncoderWriter<W> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        try_nb!(self.inner.finish());
        self.inner.get_mut().shutdown()
    }
}

impl<W: Read + Write> Read for EncoderWriter<W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.get_mut().read(buf)
    }
}

#[cfg(feature = "tokio")]
impl<W: AsyncRead + AsyncWrite> AsyncRead for EncoderWriter<W> {
}

impl<R: Read> EncoderReader<R> {
    /// Creates a new encoder which will read uncompressed data from the given
    /// stream and emit the compressed stream.
    pub fn new(r: R, level: ::Compression) -> EncoderReader<R> {
        EncoderReader {
            inner: EncoderReaderBuf::new(BufReader::new(r), level),
        }
    }
}

impl<R> EncoderReader<R> {
    /// Resets the state of this encoder entirely, swapping out the input
    /// stream for another.
    ///
    /// This function will reset the internal state of this encoder and replace
    /// the input stream with the one provided, returning the previous input
    /// stream. Future data read from this encoder will be the compressed
    /// version of `r`'s data.
    ///
    /// Note that there may be currently buffered data when this function is
    /// called, and in that case the buffered data is discarded.
    pub fn reset(&mut self, r: R) -> R {
        self.inner.data.reset();
        self.inner.obj.reset(r)
    }

    /// Acquires a reference to the underlying reader
    pub fn get_ref(&self) -> &R {
        self.inner.get_ref().get_ref()
    }

    /// Acquires a mutable reference to the underlying stream
    ///
    /// Note that mutation of the stream may result in surprising results if
    /// this encoder is continued to be used.
    pub fn get_mut(&mut self) -> &mut R {
        self.inner.get_mut().get_mut()
    }

    /// Consumes this encoder, returning the underlying reader.
    ///
    /// Note that there may be buffered bytes which are not re-acquired as part
    /// of this transition. It's recommended to only call this function after
    /// EOF has been reached.
    pub fn into_inner(self) -> R {
        self.inner.into_inner().into_inner()
    }

    /// Returns the number of bytes that have been read into this compressor.
    ///
    /// Note that not all bytes read from the underlying object may be accounted
    /// for, there may still be some active buffering.
    pub fn total_in(&self) -> u64 {
        self.inner.data.total_in()
    }

    /// Returns the number of bytes that the compressor has produced.
    ///
    /// Note that not all bytes may have been read yet, some may still be
    /// buffered.
    pub fn total_out(&self) -> u64 {
        self.inner.data.total_out()
    }
}

impl<R: Read> Read for EncoderReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncRead> AsyncRead for EncoderReader<R> {
}

impl<W: Read + Write> Write for EncoderReader<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.get_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.get_mut().flush()
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncRead + AsyncWrite> AsyncWrite for EncoderReader<R> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.get_mut().shutdown()
    }
}

impl<R: BufRead> EncoderReaderBuf<R> {
    /// Creates a new encoder which will read uncompressed data from the given
    /// stream and emit the compressed stream.
    pub fn new(r: R, level: ::Compression) -> EncoderReaderBuf<R> {
        EncoderReaderBuf {
            obj: r,
            data: Compress::new(level, false),
        }
    }
}

impl<R> EncoderReaderBuf<R> {
    /// Resets the state of this encoder entirely, swapping out the input
    /// stream for another.
    ///
    /// This function will reset the internal state of this encoder and replace
    /// the input stream with the one provided, returning the previous input
    /// stream. Future data read from this encoder will be the compressed
    /// version of `r`'s data.
    pub fn reset(&mut self, r: R) -> R {
        self.data.reset();
        mem::replace(&mut self.obj, r)
    }

    /// Acquires a reference to the underlying reader
    pub fn get_ref(&self) -> &R {
        &self.obj
    }

    /// Acquires a mutable reference to the underlying stream
    ///
    /// Note that mutation of the stream may result in surprising results if
    /// this encoder is continued to be used.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.obj
    }

    /// Consumes this encoder, returning the underlying reader.
    pub fn into_inner(self) -> R {
        self.obj
    }

    /// Returns the number of bytes that have been read into this compressor.
    ///
    /// Note that not all bytes read from the underlying object may be accounted
    /// for, there may still be some active buffering.
    pub fn total_in(&self) -> u64 {
        self.data.total_in()
    }

    /// Returns the number of bytes that the compressor has produced.
    ///
    /// Note that not all bytes may have been read yet, some may still be
    /// buffered.
    pub fn total_out(&self) -> u64 {
        self.data.total_out()
    }
}

impl<R: BufRead> Read for EncoderReaderBuf<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        zio::read(&mut self.obj, &mut self.data, buf)
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncRead + BufRead> AsyncRead for EncoderReaderBuf<R> {
}

impl<W: BufRead + Write> Write for EncoderReaderBuf<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.get_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.get_mut().flush()
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncWrite + BufRead> AsyncWrite for EncoderReaderBuf<R> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.get_mut().shutdown()
    }
}

impl<R: Read> DecoderReader<R> {
    /// Creates a new decoder which will decompress data read from the given
    /// stream.
    pub fn new(r: R) -> DecoderReader<R> {
        DecoderReader::new_with_buf(r, vec![0; 32 * 1024])
    }

    /// Same as `new`, but the intermediate buffer for data is specified.
    ///
    /// Note that the capacity of the intermediate buffer is never increased,
    /// and it is recommended for it to be large.
    pub fn new_with_buf(r: R, buf: Vec<u8>) -> DecoderReader<R> {
        DecoderReader {
            inner: DecoderReaderBuf::new(BufReader::with_buf(buf, r))
        }
    }
}

impl<R> DecoderReader<R> {
    /// Resets the state of this decoder entirely, swapping out the input
    /// stream for another.
    ///
    /// This will reset the internal state of this decoder and replace the
    /// input stream with the one provided, returning the previous input
    /// stream. Future data read from this decoder will be the decompressed
    /// version of `r`'s data.
    ///
    /// Note that there may be currently buffered data when this function is
    /// called, and in that case the buffered data is discarded.
    pub fn reset(&mut self, r: R) -> R {
        self.inner.data = Decompress::new(false);
        self.inner.obj.reset(r)
    }

    /// Acquires a reference to the underlying stream
    pub fn get_ref(&self) -> &R {
        self.inner.get_ref().get_ref()
    }

    /// Acquires a mutable reference to the underlying stream
    ///
    /// Note that mutation of the stream may result in surprising results if
    /// this encoder is continued to be used.
    pub fn get_mut(&mut self) -> &mut R {
        self.inner.get_mut().get_mut()
    }

    /// Consumes this decoder, returning the underlying reader.
    ///
    /// Note that there may be buffered bytes which are not re-acquired as part
    /// of this transition. It's recommended to only call this function after
    /// EOF has been reached.
    pub fn into_inner(self) -> R {
        self.inner.into_inner().into_inner()
    }

    /// Returns the number of bytes that the decompressor has consumed.
    ///
    /// Note that this will likely be smaller than what the decompressor
    /// actually read from the underlying stream due to buffering.
    pub fn total_in(&self) -> u64 {
        self.inner.total_in()
    }

    /// Returns the number of bytes that the decompressor has produced.
    pub fn total_out(&self) -> u64 {
        self.inner.total_out()
    }
}

impl<R: Read> Read for DecoderReader<R> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        self.inner.read(into)
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncRead> AsyncRead for DecoderReader<R> {
}

impl<W: Read + Write> Write for DecoderReader<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.get_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.get_mut().flush()
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncWrite + AsyncRead> AsyncWrite for DecoderReader<R> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.get_mut().shutdown()
    }
}

impl<R: BufRead> DecoderReaderBuf<R> {
    /// Creates a new decoder which will decompress data read from the given
    /// stream.
    pub fn new(r: R) -> DecoderReaderBuf<R> {
        DecoderReaderBuf {
            obj: r,
            data: Decompress::new(false),
        }
    }
}

impl<R> DecoderReaderBuf<R> {
    /// Resets the state of this decoder entirely, swapping out the input
    /// stream for another.
    ///
    /// This will reset the internal state of this decoder and replace the
    /// input stream with the one provided, returning the previous input
    /// stream. Future data read from this decoder will be the decompressed
    /// version of `r`'s data.
    pub fn reset(&mut self, r: R) -> R {
        self.data = Decompress::new(false);
        mem::replace(&mut self.obj, r)
    }

    /// Resets the state of this decoder's data
    ///
    /// This will reset the internal state of this decoder. It will continue
    /// reading from the same stream.
    pub fn reset_data(&mut self) {
        self.data = Decompress::new(false);
    }

    /// Acquires a reference to the underlying stream
    pub fn get_ref(&self) -> &R {
        &self.obj
    }

    /// Acquires a mutable reference to the underlying stream
    ///
    /// Note that mutation of the stream may result in surprising results if
    /// this encoder is continued to be used.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.obj
    }

    /// Consumes this decoder, returning the underlying reader.
    pub fn into_inner(self) -> R {
        self.obj
    }

    /// Returns the number of bytes that the decompressor has consumed.
    ///
    /// Note that this will likely be smaller than what the decompressor
    /// actually read from the underlying stream due to buffering.
    pub fn total_in(&self) -> u64 {
        self.data.total_in()
    }

    /// Returns the number of bytes that the decompressor has produced.
    pub fn total_out(&self) -> u64 {
        self.data.total_out()
    }
}

impl<R: BufRead> Read for DecoderReaderBuf<R> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        zio::read(&mut self.obj, &mut self.data, into)
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncRead + BufRead> AsyncRead for DecoderReaderBuf<R> {
}

impl<W: BufRead + Write> Write for DecoderReaderBuf<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.get_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.get_mut().flush()
    }
}

#[cfg(feature = "tokio")]
impl<R: AsyncWrite + BufRead> AsyncWrite for DecoderReaderBuf<R> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.get_mut().shutdown()
    }
}

impl<W: Write> DecoderWriter<W> {
    /// Creates a new decoder which will write uncompressed data to the stream.
    ///
    /// When this encoder is dropped or unwrapped the final pieces of data will
    /// be flushed.
    pub fn new(w: W) -> DecoderWriter<W> {
        DecoderWriter {
            inner: zio::Writer::new(w, Decompress::new(false)),
        }
    }

    /// Acquires a reference to the underlying writer.
    pub fn get_ref(&self) -> &W {
        self.inner.get_ref()
    }

    /// Acquires a mutable reference to the underlying writer.
    ///
    /// Note that mutating the output/input state of the stream may corrupt this
    /// object, so care must be taken when using this method.
    pub fn get_mut(&mut self) -> &mut W {
        self.inner.get_mut()
    }

    /// Resets the state of this decoder entirely, swapping out the output
    /// stream for another.
    ///
    /// This function will finish encoding the current stream into the current
    /// output stream before swapping out the two output streams.
    ///
    /// This will then reset the internal state of this decoder and replace the
    /// output stream with the one provided, returning the previous output
    /// stream. Future data written to this decoder will be decompressed into
    /// the output stream `w`.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to finish the stream, and if that I/O
    /// returns an error then that will be returned from this function.
    pub fn reset(&mut self, w: W) -> io::Result<W> {
        try!(self.inner.finish());
        self.inner.data = Decompress::new(false);
        Ok(self.inner.replace(w))
    }

    /// Attempt to finish this output stream, writing out final chunks of data.
    ///
    /// Note that this function can only be used once data has finished being
    /// written to the output stream. After this function is called then further
    /// calls to `write` may result in a panic.
    ///
    /// # Panics
    ///
    /// Attempts to write data to this stream may result in a panic after this
    /// function is called.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to finish the stream, returning any
    /// errors which happen.
    pub fn try_finish(&mut self) -> io::Result<()> {
        self.inner.finish()
    }

    /// Consumes this encoder, flushing the output stream.
    ///
    /// This will flush the underlying data stream and then return the contained
    /// writer if the flush succeeded.
    ///
    /// Note that this function may not be suitable to call in a situation where
    /// the underlying stream is an asynchronous I/O stream. To finish a stream
    /// the `try_finish` (or `shutdown`) method should be used instead. To
    /// re-acquire ownership of a stream it is safe to call this method after
    /// `try_finish` or `shutdown` has returned `Ok`.
    ///
    /// # Errors
    ///
    /// This function will perform I/O to complete this stream, and any I/O
    /// errors which occur will be returned from this function.
    pub fn finish(mut self) -> io::Result<W> {
        try!(self.inner.finish());
        Ok(self.inner.take_inner())
    }

    /// Returns the number of bytes that the decompressor has consumed for
    /// decompression.
    ///
    /// Note that this will likely be smaller than the number of bytes
    /// successfully written to this stream due to internal buffering.
    pub fn total_in(&self) -> u64 {
        self.inner.data.total_in()
    }

    /// Returns the number of bytes that the decompressor has written to its
    /// output stream.
    pub fn total_out(&self) -> u64 {
        self.inner.data.total_out()
    }
}

impl<W: Write> Write for DecoderWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(feature = "tokio")]
impl<W: AsyncWrite> AsyncWrite for DecoderWriter<W> {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        try_nb!(self.inner.finish());
        self.inner.get_mut().shutdown()
    }
}

impl<W: Read + Write> Read for DecoderWriter<W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.get_mut().read(buf)
    }
}

#[cfg(feature = "tokio")]
impl<W: AsyncRead + AsyncWrite> AsyncRead for DecoderWriter<W> {
}

#[cfg(test)]
mod tests {
    use std::io::prelude::*;

    use rand::{thread_rng, Rng};

    use deflate::{EncoderWriter, EncoderReader, DecoderReader, DecoderWriter};
    use Compression::Default;

    #[test]
    fn roundtrip() {
        let mut real = Vec::new();
        let mut w = EncoderWriter::new(Vec::new(), Default);
        let v = thread_rng().gen_iter::<u8>().take(1024).collect::<Vec<_>>();
        for _ in 0..200 {
            let to_write = &v[..thread_rng().gen_range(0, v.len())];
            real.extend(to_write.iter().map(|x| *x));
            w.write_all(to_write).unwrap();
        }
        let result = w.finish().unwrap();
        let mut r = DecoderReader::new(&result[..]);
        let mut ret = Vec::new();
        r.read_to_end(&mut ret).unwrap();
        assert!(ret == real);
    }

    #[test]
    fn drop_writes() {
        let mut data = Vec::new();
        EncoderWriter::new(&mut data, Default).write_all(b"foo").unwrap();
        let mut r = DecoderReader::new(&data[..]);
        let mut ret = Vec::new();
        r.read_to_end(&mut ret).unwrap();
        assert!(ret == b"foo");
    }

    #[test]
    fn total_in() {
        let mut real = Vec::new();
        let mut w = EncoderWriter::new(Vec::new(), Default);
        let v = thread_rng().gen_iter::<u8>().take(1024).collect::<Vec<_>>();
        for _ in 0..200 {
            let to_write = &v[..thread_rng().gen_range(0, v.len())];
            real.extend(to_write.iter().map(|x| *x));
            w.write_all(to_write).unwrap();
        }
        let mut result = w.finish().unwrap();

        let result_len = result.len();

        for _ in 0..200 {
            result.extend(v.iter().map(|x| *x));
        }

        let mut r = DecoderReader::new(&result[..]);
        let mut ret = Vec::new();
        r.read_to_end(&mut ret).unwrap();
        assert!(ret == real);
        assert_eq!(r.total_in(), result_len as u64);
    }

    #[test]
    fn roundtrip2() {
        let v = thread_rng()
                    .gen_iter::<u8>()
                    .take(1024 * 1024)
                    .collect::<Vec<_>>();
        let mut r = DecoderReader::new(EncoderReader::new(&v[..], Default));
        let mut ret = Vec::new();
        r.read_to_end(&mut ret).unwrap();
        assert_eq!(ret, v);
    }

    #[test]
    fn roundtrip3() {
        let v = thread_rng()
                    .gen_iter::<u8>()
                    .take(1024 * 1024)
                    .collect::<Vec<_>>();
        let mut w = EncoderWriter::new(DecoderWriter::new(Vec::new()), Default);
        w.write_all(&v).unwrap();
        let w = w.finish().unwrap().finish().unwrap();
        assert!(w == v);
    }

    #[test]
    fn reset_writer() {
        let v = thread_rng()
                    .gen_iter::<u8>()
                    .take(1024 * 1024)
                    .collect::<Vec<_>>();
        let mut w = EncoderWriter::new(Vec::new(), Default);
        w.write_all(&v).unwrap();
        let a = w.reset(Vec::new()).unwrap();
        w.write_all(&v).unwrap();
        let b = w.finish().unwrap();

        let mut w = EncoderWriter::new(Vec::new(), Default);
        w.write_all(&v).unwrap();
        let c = w.finish().unwrap();
        assert!(a == b && b == c);
    }

    #[test]
    fn reset_reader() {
        let v = thread_rng()
                    .gen_iter::<u8>()
                    .take(1024 * 1024)
                    .collect::<Vec<_>>();
        let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
        let mut r = EncoderReader::new(&v[..], Default);
        r.read_to_end(&mut a).unwrap();
        r.reset(&v[..]);
        r.read_to_end(&mut b).unwrap();

        let mut r = EncoderReader::new(&v[..], Default);
        r.read_to_end(&mut c).unwrap();
        assert!(a == b && b == c);
    }

    #[test]
    fn reset_decoder() {
        let v = thread_rng()
                    .gen_iter::<u8>()
                    .take(1024 * 1024)
                    .collect::<Vec<_>>();
        let mut w = EncoderWriter::new(Vec::new(), Default);
        w.write_all(&v).unwrap();
        let data = w.finish().unwrap();

        {
            let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
            let mut r = DecoderReader::new(&data[..]);
            r.read_to_end(&mut a).unwrap();
            r.reset(&data);
            r.read_to_end(&mut b).unwrap();

            let mut r = DecoderReader::new(&data[..]);
            r.read_to_end(&mut c).unwrap();
            assert!(a == b && b == c && c == v);
        }

        {
            let mut w = DecoderWriter::new(Vec::new());
            w.write_all(&data).unwrap();
            let a = w.reset(Vec::new()).unwrap();
            w.write_all(&data).unwrap();
            let b = w.finish().unwrap();

            let mut w = DecoderWriter::new(Vec::new());
            w.write_all(&data).unwrap();
            let c = w.finish().unwrap();
            assert!(a == b && b == c && c == v);
        }
    }

    #[test]
    fn zero_length_read_with_data() {
        let m = vec![3u8; 128 * 1024 + 1];
        let mut c = EncoderReader::new(&m[..], ::Compression::Default);

        let mut result = Vec::new();
        c.read_to_end(&mut result).unwrap();

        let mut d = DecoderReader::new(&result[..]);
        let mut data = Vec::new();
        assert!(d.read(&mut data).unwrap() == 0);
    }

    #[test]
    fn qc_reader() {
        ::quickcheck::quickcheck(test as fn(_) -> _);

        fn test(v: Vec<u8>) -> bool {
            let mut r = DecoderReader::new(EncoderReader::new(&v[..], Default));
            let mut v2 = Vec::new();
            r.read_to_end(&mut v2).unwrap();
            v == v2
        }
    }

    #[test]
    fn qc_writer() {
        ::quickcheck::quickcheck(test as fn(_) -> _);

        fn test(v: Vec<u8>) -> bool {
            let mut w = EncoderWriter::new(DecoderWriter::new(Vec::new()), Default);
            w.write_all(&v).unwrap();
            v == w.finish().unwrap().finish().unwrap()
        }
    }
}
