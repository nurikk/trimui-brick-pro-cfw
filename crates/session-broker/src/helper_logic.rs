use std::{process::Command, thread, time::Duration};

pub fn run(arguments: Vec<String>) {
    let grandchild = arguments.iter().any(|argument| argument == "--grandchild");
    if !grandchild {
        wait_for_barrier();
    }
    if grandchild {
        thread::sleep(Duration::from_secs(5));
        return;
    }
    let scenario = arguments
        .windows(2)
        .find(|pair| pair[0] == "--scenario")
        .map(|pair| pair[1].as_str())
        .unwrap_or("success");
    match scenario {
        "success" => thread::sleep(Duration::from_millis(5)),
        "nonzero" => std::process::exit(7),
        "signal" => std::process::abort(),
        "grandchild" => {
            let executable = std::env::current_exe().expect("helper executable");
            let mut child = Command::new(executable)
                .arg("--helper")
                .arg("--grandchild")
                .spawn()
                .expect("grandchild");
            thread::sleep(Duration::from_secs(5));
            let _ = child.kill();
            let _ = child.wait();
        }
        "timeout" | "cancel" => thread::sleep(Duration::from_secs(5)),
        _ => std::process::exit(9),
    }
}

fn wait_for_barrier() {
    let Ok(value) = std::env::var("BROKER_BARRIER_FD") else {
        return;
    };
    let Ok(fd) = value.parse::<libc::c_int>() else {
        std::process::exit(10);
    };
    let mut byte = [0u8; 1];
    let count = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
    unsafe { libc::close(fd) };
    if count != 1 {
        std::process::exit(10);
    }
}
