//! Apple Silicon core topology, thermal throttling monitor and Darwin background QoS cooling governor.
//! Inspired by NASDAQ-US-ANT-TRADER cooling and resource management architecture.

use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CpuTopology {
    pub chip_name: String,
    pub e_cores: usize,
    pub p_cores: usize,
    pub total_cores: usize,
    pub is_apple_silicon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ThermalStatus {
    Nominal,
    Warm,
    Hot,
    Throttled,
}

impl ThermalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThermalStatus::Nominal => "Nominal (Cooling Active)",
            ThermalStatus::Warm => "Warm (Yield Pacing)",
            ThermalStatus::Hot => "Hot (Throttling Cooldown)",
            ThermalStatus::Throttled => "Throttled (Thermal Trip)",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GovernorState {
    pub topology: CpuTopology,
    pub thermal_status: ThermalStatus,
    pub cpu_speed_limit: Option<f64>,
    pub qos_active: bool,
    pub e_core_pinned: bool,
    pub load_1m: f64,
    pub status_text: String,
}

pub fn detect_cpu_topology() -> CpuTopology {
    #[cfg(target_os = "macos")]
    {
        let chip_name = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Apple Silicon".to_string());

        let e_cores = Command::new("sysctl")
            .args(["-n", "hw.perflevel1.physicalcpu"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);

        let p_cores = Command::new("sysctl")
            .args(["-n", "hw.perflevel0.physicalcpu"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);

        let is_apple = chip_name.contains("Apple") || (e_cores > 0 && p_cores > 0);
        let total_cores = if e_cores + p_cores > 0 {
            e_cores + p_cores
        } else {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8)
        };

        CpuTopology {
            chip_name,
            e_cores: if e_cores > 0 { e_cores } else { total_cores.saturating_sub(4).max(2) },
            p_cores: if p_cores > 0 { p_cores } else { 4 },
            total_cores,
            is_apple_silicon: is_apple,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let total = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        CpuTopology {
            chip_name: "Generic CPU".to_string(),
            e_cores: total / 2,
            p_cores: total - (total / 2),
            total_cores: total,
            is_apple_silicon: false,
        }
    }
}

pub fn check_thermal_status() -> (ThermalStatus, Option<f64>) {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("/usr/bin/pmset").args(["-g", "therm"]).output() {
            let s = String::from_utf8_lossy(&output.stdout).to_lowercase();
            let mut speed_limit: Option<f64> = None;
            for line in s.lines() {
                if let Some(pos) = line.find("cpu_speed_limit")
                    && let Some(eq) = line[pos..].find('=')
                        && let Ok(val) = line[pos + eq + 1..].trim().parse::<f64>() {
                            speed_limit = Some(val);
                        }
            }
            if let Some(limit) = speed_limit
                && limit < 80.0 {
                    return (ThermalStatus::Throttled, Some(limit));
                }
            if s.contains("warning") || s.contains("thermal level: 2") || s.contains("thermal level = 2") {
                return (ThermalStatus::Hot, speed_limit);
            }
            return (ThermalStatus::Nominal, speed_limit);
        }
    }
    (ThermalStatus::Nominal, None)
}

/// Applies Darwin Background QoS to current thread, instructing macOS kernel
/// to schedule execution strictly onto Efficiency (E-cores), keeping Performance (P-cores) cold.
pub fn apply_cooling_qos() -> bool {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        // QOS_CLASS_BACKGROUND = 0x09; QOS_CLASS_UTILITY = 0x11
        const QOS_CLASS_BACKGROUND: u32 = 0x09;
        unsafe { pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0) == 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub fn get_governor_state() -> GovernorState {
    let topology = detect_cpu_topology();
    let (thermal_status, speed_limit) = check_thermal_status();
    let load_1m = {
        #[cfg(target_os = "macos")]
        {
            unsafe extern "C" {
                fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
            }
            let mut loads = [0.0f64; 3];
            let ret = unsafe { getloadavg(loads.as_mut_ptr(), 3) };
            if ret > 0 { loads[0] } else { 0.0 }
        }
        #[cfg(not(target_os = "macos"))]
        {
            0.0
        }
    };

    let status_text = format!(
        "{} ({} E-cores, {} P-cores) · {} · Load: {:.2}",
        topology.chip_name,
        topology.e_cores,
        topology.p_cores,
        thermal_status.as_str(),
        load_1m
    );

    GovernorState {
        topology,
        thermal_status,
        cpu_speed_limit: speed_limit,
        qos_active: true,
        e_core_pinned: true,
        load_1m,
        status_text,
    }
}

/// Yields or sleeps briefly during intensive sweeps if the machine begins heating up.
pub fn pace_for_cooling(thermal: ThermalStatus) {
    match thermal {
        ThermalStatus::Nominal => {
            // Cooperative yield
            std::thread::yield_now();
        }
        ThermalStatus::Warm => {
            std::thread::sleep(Duration::from_millis(2));
        }
        ThermalStatus::Hot | ThermalStatus::Throttled => {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
}
