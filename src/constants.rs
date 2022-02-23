pub const HELLO_MSG: &[u8] = b"PING";
pub const EMPTY_MSG: &[u8] = b"";
pub const ACK_MSG: &[u8] = b"PONG";
pub const MSG_BUFFER_LENGTH: usize = 16;
// @TODO Make it a configurable option
pub const MAX_OPEN_FILES_COUNT: u32 = 50;
