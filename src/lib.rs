//! An implementation of an event loop backed by a Win32 window.

#![cfg(windows)]

#[macro_use]
extern crate log;

#[macro_use]
extern crate lazy_static;

extern crate winapi;

mod util;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use winapi::shared::minwindef::{FALSE, LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::HWND;

use winapi::um::processthreadsapi::GetCurrentThreadId;
use winapi::um::winuser::*;

#[derive(Debug)]
enum HwndLoopCommand<CommandType: Send + std::fmt::Debug> {
  Terminate,
  UserCommand(CommandType),
}

/// Send and Sync wrapper for [`HWND`].
///
/// [`HWND`] is a raw pointer, which can't be made [`Send`] or [`Sync`] directly, so wrap it in a helper type.
#[derive(Clone)]
pub struct HwndWrapper(pub HWND);
unsafe impl Send for HwndWrapper {}
unsafe impl Sync for HwndWrapper {}

/// Callbacks called by a [`HwndLoop`].
#[allow(unused_variables)]
pub trait HwndLoopCallbacks<CommandType: std::fmt::Debug>: Send {
  /// Called on the handler thread just before the [`HwndLoop`] starts.
  fn set_up(&mut self, hwnd: HWND) {}

  /// Called on the handler thread just before the [`HwndLoop`] terminates.
  ///
  /// Note that if you need to wait for a message to finish tearing down, it is too late by this
  /// point. The HWND and thread will be destroyed immediateliy after this function returns.
  fn tear_down(&mut self, hwnd: HWND) {}

  /// Handle a Windows message.
  ///
  /// Note that most messages need to have [`DefWindowProcA`] called on them for cleanup.
  fn handle_message(&mut self, hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcA(hwnd, msg, w, l) }
  }

  /// Handle a command sent via [`HwndLoop::send_command`].
  fn handle_command(&mut self, hwnd: HWND, cmd: CommandType) {}
}

/// An event loop backed by a Win32 window and thread.
///
/// A [`HwndLoop`] consists of a message window and handler thread on which all callbacks happen.
pub struct HwndLoop<CommandType: Send + std::fmt::Debug + 'static> {
  hwnd: HwndWrapper,
  terminated: Arc<AtomicBool>,
  command_queue: Arc<Mutex<VecDeque<HwndLoopCommand<CommandType>>>>,
  join_handle: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
  flush_requests: Arc<Mutex<Vec<std::sync::mpsc::Sender<()>>>>,
}

#[repr(C)]
struct HwndLoopWndExtra<CommandType: Send + std::fmt::Debug> {
  callbacks: *mut Box<HwndLoopCallbacks<CommandType>>,
}

impl<CommandType: Send + std::fmt::Debug> HwndLoopWndExtra<CommandType> {
  unsafe fn from_hwnd(hwnd: HWND) -> *mut HwndLoopWndExtra<CommandType> {
    let ptr = GetWindowLongPtrA(hwnd, 0);
    std::mem::transmute(ptr)
  }
}

lazy_static! {
  static ref WM_HWNDLOOP_INIT: u32 = {
    let msg = unsafe { RegisterWindowMessageA(b"WM_HWNDLOOP_INIT\0".as_ptr() as *const i8) };
    assert_ne!(0, msg);
    msg
  };
  static ref WM_HWNDLOOP_COMMAND: u32 = {
    let msg = unsafe { RegisterWindowMessageA(b"WM_HWNDLOOP_COMMAND\0".as_ptr() as *const i8) };
    assert_ne!(0, msg);
    msg
  };
  static ref WM_HWNDLOOP_FLUSH: u32 = {
    let msg = unsafe { RegisterWindowMessageA(b"WM_HWNDLOOP_FLUSH\0".as_ptr() as *const i8) };
    assert_ne!(0, msg);
    msg
  };
}

impl<CommandType: Send + std::fmt::Debug + 'static> HwndLoop<CommandType> {
  /// Create a new [`HwndLoop`].
  pub fn new(mut callbacks: Box<HwndLoopCallbacks<CommandType>>) -> HwndLoop<CommandType> {
    let (tx, rx) = channel();
    let join_handle = std::thread::spawn(move || {
      let class_name = util::to_utf16(&format!("RawInputRS{}", unsafe { GetCurrentThreadId() }));
      let wndclass = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as UINT,
        style: 0,
        lpfnWndProc: Some(HwndLoop::<CommandType>::wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: std::mem::size_of::<*mut HwndLoopWndExtra<CommandType>>() as i32,
        hInstance: util::get_module_handle(),
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null_mut(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: std::ptr::null_mut(),
      };

      let window_class = unsafe { RegisterClassExW(&wndclass) };
      if window_class == 0 {
        panic!("RegisterClassExW failed: {}", std::io::Error::last_os_error());
      }

      let hwnd = unsafe {
        CreateWindowExW(
          WS_EX_NOREDIRECTIONBITMAP,
          util::atom_to_lpwstr(window_class),
          util::to_utf16("rawinput window").as_ptr(),
          0,
          CW_USEDEFAULT,
          CW_USEDEFAULT,
          CW_USEDEFAULT,
          CW_USEDEFAULT,
          HWND_MESSAGE,
          std::ptr::null_mut(),
          util::get_module_handle(),
          std::ptr::null_mut(),
        )
      };

      if hwnd == std::ptr::null_mut() {
        panic!("CreateWindowExW failed");
      }

      let command_queue = Arc::new(Mutex::new(VecDeque::new()));
      let flush_requests = Arc::new(Mutex::new(Vec::<std::sync::mpsc::Sender<()>>::new()));

      let mut msg = unsafe { std::mem::uninitialized() };

      let result = unsafe { PostMessageW(hwnd, *WM_HWNDLOOP_INIT, 0, 1) };
      if result == 0 {
        panic!(
          "failed to PostMessageW during message window startup: {}",
          std::io::Error::last_os_error()
        );
      }

      callbacks.set_up(hwnd);

      // Set up the callbacks to be called from wnd_proc.
      let raw_cb = Box::into_raw(Box::new(callbacks));
      let wnd_extra = Box::into_raw(Box::new(HwndLoopWndExtra { callbacks: raw_cb }));
      unsafe { SetWindowLongPtrA(hwnd, 0, std::mem::transmute(wnd_extra)) };

      'eventloop: loop {
        let result = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if result <= 0 {
          panic!("GetMessageW failed");
        }

        // We're started, time to return the result.
        if msg.message == *WM_HWNDLOOP_INIT {
          tx.send((HwndWrapper(hwnd), command_queue.clone(), flush_requests.clone()))
            .unwrap();
        } else if msg.message == *WM_HWNDLOOP_COMMAND {
          // Only process commands when we receive a poke, to ensure that we maintain ordering.
          let mut queue = command_queue.lock().unwrap();
          if !queue.is_empty() {
            let cmd = queue.pop_front().unwrap();
            trace!("HwndLoop received command: {:?}", cmd);
            match cmd {
              HwndLoopCommand::Terminate => {
                break 'eventloop;
              }

              HwndLoopCommand::UserCommand(cmd) => {
                unsafe { (*raw_cb).handle_command(hwnd, cmd) };
              }
            }
          }
        } else if msg.message == *WM_HWNDLOOP_FLUSH {
          let mut reqs = flush_requests.lock().unwrap();
          (*reqs).pop().unwrap().send(()).unwrap();
        } else {
          unsafe { DispatchMessageW(&msg) };
        }
      }

      unsafe { (*raw_cb).tear_down(hwnd) };

      // Remove the callbacks from the window.
      unsafe { SetWindowLongPtrA(hwnd, 0, 0) };

      // Destroy the callbacks.
      unsafe { Box::from_raw(raw_cb) };

      // Destroy the window.
      unsafe { assert_ne!(FALSE, DestroyWindow(hwnd)) };

      // Destroy the window class.
      unsafe {
        assert_ne!(
          FALSE,
          UnregisterClassW(util::atom_to_lpwstr(window_class), util::get_module_handle())
        )
      };
    });

    let (hwnd, command_queue, flush_requests) = rx.recv().unwrap();
    HwndLoop {
      terminated: Arc::new(AtomicBool::from(false)),
      hwnd,
      command_queue,
      join_handle: Arc::new(Mutex::new(Some(join_handle))),
      flush_requests,
    }
  }

  unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    let wnd_extra = HwndLoopWndExtra::<CommandType>::from_hwnd(hwnd);
    if wnd_extra == std::ptr::null_mut() {
      return DefWindowProcA(hwnd, msg, w, l);
    }

    (*(*wnd_extra).callbacks).handle_message(hwnd, msg, w, l)
  }

  fn send_command_internal(&self, cmd: HwndLoopCommand<CommandType>) {
    let mut queue = self.command_queue.lock().unwrap();
    queue.push_back(cmd);
    let result = unsafe { PostMessageW(self.hwnd.0, *WM_HWNDLOOP_COMMAND, 0, 1) };
    if result == FALSE {
      panic!("PostMessageW failed: {}", std::io::Error::last_os_error());
    }
  }

  /// Send a command to a [`HwndLoop`], to be handled by [`HwndLoopCallbacks::handle_command`] on
  /// the handler thread.
  pub fn send_command(&self, cmd: CommandType) {
    trace!("HwndLoop sending user command: {:?}", cmd);
    self.send_command_internal(HwndLoopCommand::UserCommand(cmd))
  }

  /// Wait until all previously enqueued messages have been processed.
  pub fn flush(&self) {
    let (tx, rx) = channel();
    let mut requests = self.flush_requests.lock().unwrap();

    (*requests).push(tx);
    let result = unsafe { PostMessageW(self.hwnd.0, *WM_HWNDLOOP_FLUSH, 0, 0) };
    if result == FALSE {
      panic!("PostMessageW failed: {}", std::io::Error::last_os_error());
    }

    drop(requests);

    rx.recv().unwrap();
  }

  fn terminate(&self) {
    let terminated = self.terminated.swap(true, Ordering::SeqCst);
    if !terminated {
      self.send_command_internal(HwndLoopCommand::Terminate);
      let mut opt = self.join_handle.lock().unwrap();
      let join_handle = std::mem::replace(&mut *opt, None);
      join_handle.unwrap().join().unwrap();
    }
  }
}

impl<CommandType: Send + std::fmt::Debug + 'static> Drop for HwndLoop<CommandType> {
  fn drop(&mut self) {
    self.terminate();
  }
}
