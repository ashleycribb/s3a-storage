use bytemuck::{Pod, Zeroable};

/// Ultra-Compact Quantized Wearable Telemetry Record (16 bytes) for Heart Rate / IMU / PPG / Gyro in Earbuds & Smart Rings.
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct CompactWearableRecord {
    pub timestamp_sec: u32, // Relative seconds timestamp (fits >130 years)
    pub sensor_id: u8,      // Sensor ID (0=PPG HeartRate, 1=Accel, 2=Gyro, 3=Temp, 4=SpO2)
    pub metric_id: u8,      // Metric ID
    pub quantized_val: i16, // INT16 scaled value (e.g. HeartRate * 256.0, Temp * 100.0)
    pub quality_flags: u16, // Signal quality, Motion artifact bits
    pub _reserved: [u8; 6],
}

impl CompactWearableRecord {
    pub fn new(timestamp_sec: u32, sensor_id: u8, metric_id: u8, float_value: f32) -> Self {
        Self {
            timestamp_sec,
            sensor_id,
            metric_id,
            quantized_val: (float_value * 256.0) as i16,
            quality_flags: 0x0001,
            _reserved: [0u8; 6],
        }
    }

    pub fn float_value(&self) -> f32 {
        (self.quantized_val as f32) / 256.0
    }
}
