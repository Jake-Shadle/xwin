#[link(name = "KERNEL32")]
extern "system" {
    #[link_name = "CloseHandle"]
    pub fn close_handle(handle: u32) -> i32;
}

fn main() {}
