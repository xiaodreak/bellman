use fs2::FileExt;
use log::{debug, info, warn};
use std::fs::File;
use std::path::PathBuf;

const GPU_LOCK_NAME: &str = "bellman.gpu.lock";
const PRIORITY_LOCK_NAME: &str = "bellman.priority.lock";
fn tmp_path(filename: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(filename);
    p
}

/// `GPULock` prevents two kernel objects to be instantiated simultaneously.
#[derive(Debug)]
pub struct GPULock(File);
impl GPULock {
    pub fn lock() -> GPULock {
        debug!("Acquiring GPU lock...");
        let f = File::create(tmp_path(GPU_LOCK_NAME)).unwrap();
        f.lock_exclusive().unwrap();
        debug!("GPU lock acquired!");
        GPULock(f)
    }
}
impl Drop for GPULock {
    fn drop(&mut self) {
        debug!("GPU lock released!");
    }
}

/// `PrioriyLock` is like a flag. When acquired, it means a high-priority process
/// needs to acquire the GPU really soon. Acquiring the `PriorityLock` is like
/// signaling all other processes to release their `GPULock`s.
/// Only one process can have the `PriorityLock` at a time.
#[derive(Debug)]
pub struct PriorityLock(File);
impl PriorityLock {
    pub fn lock() -> PriorityLock {
        debug!("Acquiring priority lock...");
        let f = File::create(tmp_path(PRIORITY_LOCK_NAME)).unwrap();
        f.lock_exclusive().unwrap();
        debug!("Priority lock acquired!");
        PriorityLock(f)
    }
    pub fn should_break(priority: bool) -> bool {
        !priority
            && File::create(tmp_path(PRIORITY_LOCK_NAME))
                .unwrap()
                .try_lock_exclusive()
                .is_err()
    }
}
impl Drop for PriorityLock {
    fn drop(&mut self) {
        debug!("Priority lock released!");
    }
}

use super::error::{GPUError, GPUResult};
use super::fft::FFTKernel;
use super::multiexp::MultiexpKernel;
use crate::domain::create_fft_kernel;
use crate::multiexp::create_multiexp_kernel;
use paired::Engine;

macro_rules! locked_kernel {
    ($class:ident, $kern:ident, $func:ident) => {
        pub struct $class<E>
        where
            E: Engine,
        {
            priority: bool,
            kernel: Option<$kern<E>>,
        }

        impl<E> $class<E>
        where
            E: Engine,
        {
            pub fn new(priority: bool) -> $class<E> {
                $class::<E> {
                    priority,
                    kernel: None,
                }
            }

            fn free(&mut self) {
                if let Some(_kernel) = self.kernel.take() {
                    warn!("GPU acquired by a high priority process! Freeing up kernels...");
                }
            }

            pub fn with<F, R>(&mut self, f: F) -> GPUResult<R>
            where
                F: FnOnce(&mut $kern<E>) -> GPUResult<R>,
            {
                if PriorityLock::should_break(self.priority) {
                    self.free();
                } else if self.kernel.is_none() {
                    info!("GPU is available!");
                    self.kernel = $func::<E>(self.priority);
                }

                if let Some(ref mut k) = self.kernel {
                    let res = f(k);
                    if let Err(GPUError::GPUTaken) = res {
                        self.free();
                    }
                    res
                } else {
                    Err(GPUError::GPUTaken)
                }
            }
        }
    };
}

locked_kernel!(LockedFFTKernel, FFTKernel, create_fft_kernel);
locked_kernel!(LockedMultiexpKernel, MultiexpKernel, create_multiexp_kernel);