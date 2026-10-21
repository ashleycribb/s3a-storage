use bytemuck::{Pod, Zeroable};

/// ROBOTICS PLATFORM KINEMATIC & IMU STATE RECORD (72 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct RoboticsKinematicRecord {
    pub timestamp_us: u64,          // Microsecond-precision timestamp
    pub robot_id: u32,              // Robot / Joint / Actuator ID
    pub joint_mask: u32,            // Joint bitmask
    pub position_xyz: [f32; 3],     // 3D Position [X, Y, Z] in meters
    pub orientation_quat: [f32; 4], // Quaternion [W, X, Y, Z]
    pub linear_velocity: [f32; 3],  // Linear Velocity [Vx, Vy, Vz]
    pub angular_velocity: [f32; 3], // Angular Velocity [Wx, Wy, Wz]
    pub _padding: u32,
}

impl RoboticsKinematicRecord {
    pub fn new(
        timestamp_us: u64,
        robot_id: u32,
        position_xyz: [f32; 3],
        orientation_quat: [f32; 4],
        linear_velocity: [f32; 3],
        angular_velocity: [f32; 3],
    ) -> Self {
        Self {
            timestamp_us,
            robot_id,
            joint_mask: 0xFFFF,
            position_xyz,
            orientation_quat,
            linear_velocity,
            angular_velocity,
            _padding: 0,
        }
    }
}
