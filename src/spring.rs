#![allow(dead_code)]

/// A second-order spring-mass-damper system for smooth UI animations.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    /// Tension or stiffness constant (k). A higher value makes the spring snappier.
    pub stiffness: f32,
    /// Damping coefficient (c). A higher value reduces oscillation.
    pub damping: f32,
}

impl Default for Spring {
    fn default() -> Self {
        // Standard, slightly underdamped / critically damped spring parameters
        Self {
            stiffness: 170.0,
            damping: 26.0,
        }
    }
}

impl Spring {
    pub fn new(stiffness: f32, damping: f32) -> Self {
        Self { stiffness, damping }
    }

    /// Simulates the spring for a given duration `dt` (in seconds).
    /// Uses sub-ticking with semi-implicit Euler integration to maintain mathematical
    /// stability even for large time steps.
    ///
    /// Returns the new `(position, velocity)`.
    pub fn update(&self, current: f32, velocity: f32, target: f32, dt: f32) -> (f32, f32) {
        if dt <= 0.0 {
            return (current, velocity);
        }

        let mut curr_pos = current;
        let mut curr_vel = velocity;

        // Substep duration (1 ms) to ensure numerical integration doesn't explode.
        const SUBSTEP: f32 = 0.001;
        let mut remaining_time = dt;

        while remaining_time > 0.0 {
            let step = remaining_time.min(SUBSTEP);

            // Hooke's Law: F = -k * dx
            // Damping force: F_d = -c * v
            // Total acceleration (assuming mass m = 1.0): a = -k * (x - target) - c * v
            let acceleration = -self.stiffness * (curr_pos - target) - self.damping * curr_vel;

            // Semi-implicit Euler integration
            curr_vel += acceleration * step;
            curr_pos += curr_vel * step;

            remaining_time -= step;
        }

        // Snap to target if we're extremely close and moving very slowly
        if (curr_pos - target).abs() < 1e-3 && curr_vel.abs() < 1e-3 {
            (target, 0.0)
        } else {
            (curr_pos, curr_vel)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spring_convergence() {
        let spring = Spring::default();
        let mut pos = 0.0;
        let mut vel = 0.0;
        let target = 100.0;

        // Run the simulation for 2 seconds (with 60 FPS increments)
        let dt = 1.0 / 60.0;
        for _ in 0..120 {
            let (new_pos, new_vel) = spring.update(pos, vel, target, dt);
            pos = new_pos;
            vel = new_vel;
        }

        // After 2 seconds, the spring should have converged very close to the target
        assert!((pos - target).abs() < 0.1, "Spring did not converge: pos={}, target={}", pos, target);
        assert!(vel.abs() < 0.1, "Spring velocity did not settle: vel={}", vel);
    }

    #[test]
    fn test_spring_no_dt() {
        let spring = Spring::default();
        let (pos, vel) = spring.update(10.0, 5.0, 100.0, 0.0);
        assert_eq!(pos, 10.0);
        assert_eq!(vel, 5.0);
    }
}
