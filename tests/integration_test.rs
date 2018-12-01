extern crate hwndloop;
extern crate winapi;

#[cfg(test)]
mod test {
  use hwndloop::*;

  use std::collections::VecDeque;
  use std::sync::mpsc::{channel, Sender};

  use winapi::shared::minwindef::{FALSE, LPARAM, LRESULT, UINT, WPARAM};
  use winapi::shared::windef::HWND;
  use winapi::um::winuser::{DefWindowProcW, PostMessageA, WM_USER};

  #[derive(Debug)]
  enum TestCommand {
    Push(i32),
    Pop(Sender<Option<i32>>),
    GetHWND(Sender<HWNDWrapper>),
  }

  struct Test {
    queue: VecDeque<i32>,
  }

  impl Test {
    fn new() -> Test {
      Test { queue: VecDeque::new() }
    }
  }

  impl HwndLoopCallbacks<TestCommand> for Test {
    fn set_up(&mut self, _hwnd: HWND) {}
    fn tear_down(&mut self, _hwnd: HWND) {}

    fn handle_message(&mut self, hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
      if msg == WM_USER {
        self.queue.push_back(w as i32);
      }

      unsafe { DefWindowProcW(hwnd, msg, w, l) }
    }

    fn handle_command(&mut self, hwnd: HWND, cmd: TestCommand) {
      match cmd {
        TestCommand::Push(i) => self.queue.push_back(i),
        TestCommand::Pop(tx) => tx.send(self.queue.pop_front()).unwrap(),
        TestCommand::GetHWND(tx) => tx.send(HWNDWrapper(hwnd)).unwrap(),
      }
    }
  }

  #[test]
  fn smoke() {
    let hwndloop = hwndloop::HwndLoop::new(Box::new(Test::new()));
    hwndloop.send_command(TestCommand::Push(1));
    let (tx, rx) = channel();
    hwndloop.send_command(TestCommand::Pop(tx));
    assert_eq!(Some(1), rx.recv().unwrap());
  }

  #[test]
  fn winmsg() {
    let hwndloop = hwndloop::HwndLoop::new(Box::new(Test::new()));
    let (tx, rx) = channel();
    hwndloop.send_command(TestCommand::GetHWND(tx));

    let hwnd = rx.recv().unwrap();
    assert_ne!(FALSE, unsafe { PostMessageA(hwnd.0, WM_USER, 123 as WPARAM, 0) });

    hwndloop.flush();

    let (tx, rx) = channel();
    hwndloop.send_command(TestCommand::Pop(tx));
    assert_eq!(Some(123), rx.recv().unwrap());
  }

  #[test]
  fn ordering() {
    let hwndloop = hwndloop::HwndLoop::new(Box::new(Test::new()));
    let (tx, rx) = channel();
    hwndloop.send_command(TestCommand::GetHWND(tx));

    let hwnd = rx.recv().unwrap();

    let (begin, end) = (0, 65536);
    for i in begin..end {
      if i % 2 == 0 {
        hwndloop.send_command(TestCommand::Push(i));
      } else {
        assert_ne!(FALSE, unsafe { PostMessageA(hwnd.0, WM_USER, i as WPARAM, 0) });
      }
    }

    for i in begin..end {
      let (tx, rx) = channel();
      hwndloop.send_command(TestCommand::Pop(tx));
      assert_eq!(Some(i), rx.recv().unwrap());
    }
  }
}
