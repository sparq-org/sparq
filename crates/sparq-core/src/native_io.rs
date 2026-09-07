// [GPT-6] Share native codecs between directories and a streamed archive.
use std::io::{self, BufWriter, Write};
use std::path::Path;

pub(crate) trait NativeSink {
    type Writer: Write;
    fn component<T>(
        &mut self,
        name: &str,
        write: impl FnOnce(&mut Self::Writer) -> io::Result<T>,
    ) -> io::Result<T>;
}

pub(crate) struct DirectorySink<'a>(pub(crate) &'a Path);

impl NativeSink for DirectorySink<'_> {
    type Writer = BufWriter<std::fs::File>;
    fn component<T>(
        &mut self,
        name: &str,
        write: impl FnOnce(&mut Self::Writer) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut writer = BufWriter::new(std::fs::File::create(self.0.join(name))?);
        let result = write(&mut writer)?;
        writer.flush()?;
        Ok(result)
    }
}
