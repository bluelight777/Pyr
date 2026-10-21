//! Bug report 35, its cause shown on any graphics card: what a frame hands the graphics card's
//! queue (`write_buffer`, as egui does with a frame's shapes) waits there until the next
//! submit. A frame that can't be shown (minimized on Windows: the surface is "outdated") never
//! submits, so frame after frame piles up; submitting nothing at all (what Pyr now does then)
//! sends them on, and they're let go.
//!
//!   cargo run --release -p pyr --example wgpu_pending
//!
//! Prints the memory the process holds after 300 frames of 3 MB each, without and with the
//! empty submit after every frame.

use eframe::wgpu;

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()))
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

fn main() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::from_env_or_default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("an adapter");
    println!("adapter: {:?}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a device");
    const FRAME: u64 = 3 << 20;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("frame"), size: FRAME, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let data = vec![7u8; FRAME as usize];
    for flush in [false, true] {
        let before = rss_mb();
        for _ in 0..300 {
            // A frame's shapes, handed over (egui-wgpu's `update_buffers`)...
            queue.write_buffer(&buffer, 0, &data);
            // ...then the surface can't be had, and the frame is skipped.
            if flush {
                queue.submit(std::iter::empty());
            }
        }
        let after = rss_mb();
        println!("{}: {:.0} MB held after 300 skipped frames (from {:.0} MB)", if flush { "with the empty submit (0.11.0)" } else { "as before (0.10.0)" }, after, before);
        // (Let what's queued go before the next run.)
        queue.submit(std::iter::empty());
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
}
