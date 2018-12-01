use winapi::shared::minwindef::{ATOM, HINSTANCE};
use winapi::um::winnt::LPWSTR;

extern "C" {
  pub static __ImageBase: u8;
}

pub fn get_module_handle() -> HINSTANCE {
  unsafe { &__ImageBase as *const u8 as HINSTANCE }
}

pub fn atom_to_lpwstr(atom: ATOM) -> LPWSTR {
  // The atom must be in the low-order word of lpClassName; the high-order word must be zero.
  atom as usize as LPWSTR
}

pub fn to_utf16(s: &str) -> Vec<u16> {
  s.encode_utf16().chain(Some(0).into_iter()).collect()
}
