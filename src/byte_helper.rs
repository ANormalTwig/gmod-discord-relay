use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct ByteReadStringError;

impl Error for ByteReadStringError {}

impl fmt::Display for ByteReadStringError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Error reading byte string")
	}
}

use String;

pub trait ByteReading {
	fn read_string(&self, start: usize) -> Result<(String, usize), ByteReadStringError>;
}

impl ByteReading for [u8] {
	fn read_string(&self, start: usize) -> Result<(String, usize), ByteReadStringError> {
		if start >= self.len() {
			return Err(ByteReadStringError);
		}

		let mut buf = vec![0; self.len() - start];
		let mut i = 0;
		for b in &self[start..] {
			let b = *b;
			buf[i] = b;
			i += 1;
			if b == 0 {
				break;
			}
		}

		return match String::from_utf8(buf[..i].to_vec()) {
			Ok(v) => Ok((v, i)),
			Err(_e) => Err(ByteReadStringError),
		};
	}
}
