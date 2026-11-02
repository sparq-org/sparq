use crate::{Error, Result};

pub(crate) fn push_field(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let len = u32::try_from(bytes.len()).map_err(|_| Error::InputTooLong)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

pub(crate) fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
    let field = input.get(..len).ok_or(Error::MalformedInput)?;
    *input = &input[len..];
    Ok(field)
}

pub(crate) fn field<'a>(input: &mut &'a [u8]) -> Result<&'a [u8]> {
    let len = u32::from_be_bytes(
        take(input, 4)?
            .try_into()
            .map_err(|_| Error::MalformedInput)?,
    );
    take(
        input,
        usize::try_from(len).map_err(|_| Error::MalformedInput)?,
    )
}

pub(crate) fn prefix(input: &mut &[u8], magic: &[u8; 4]) -> Result<()> {
    if take(input, 4)? != magic {
        return Err(Error::MalformedInput);
    }
    if take(input, 1)? != [1] {
        return Err(Error::UnsupportedVersion);
    }
    Ok(())
}
