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

        let x0 = current - target;
        let v0 = velocity;

        // Stiffness k, Damping c
        // Damping ratio zeta = c / (2 * sqrt(k))
        // Undamped angular frequency omega_0 = sqrt(k)
        let k = self.stiffness;
        let c = self.damping;

        if k <= 0.0 {
            // Unbound / unstable system, just basic linear drift or snap to target
            return (target, 0.0);
        }

        let omega_0 = k.sqrt();
        let zeta = c / (2.0 * omega_0);

        let (curr_pos, curr_vel) = if zeta < 0.9999 {
            // Underdamped
            let omega_d = omega_0 * (1.0 - zeta * zeta).sqrt();
            let exp = (-zeta * omega_0 * dt).exp();
            let cos_val = (omega_d * dt).cos();
            let sin_val = (omega_d * dt).sin();

            // x(t) = exp * (x0 * cos + B * sin)
            // where B = (v0 + zeta * omega_0 * x0) / omega_d
            let b = (v0 + zeta * omega_0 * x0) / omega_d;
            let x_t = exp * (x0 * cos_val + b * sin_val);

            // v(t) = exp * (v0 * cos - ((omega_0^2 * x0 + zeta * omega_0 * v0) / omega_d) * sin)
            let v_coeff = (omega_0 * omega_0 * x0 + zeta * omega_0 * v0) / omega_d;
            let v_t = exp * (v0 * cos_val - v_coeff * sin_val);

            (x_t + target, v_t)
        } else if zeta > 1.0001 {
            // Overdamped
            let beta = omega_0 * (zeta * zeta - 1.0).sqrt();
            let r1 = -zeta * omega_0 + beta;
            let r2 = -zeta * omega_0 - beta;

            let exp1 = (r1 * dt).exp();
            let exp2 = (r2 * dt).exp();

            // A = (v0 - r2 * x0) / (2 * beta)
            // B = (-v0 + (beta - zeta * omega_0) * x0) / (2 * beta)
            let div = 2.0 * beta;
            let a = (v0 - r2 * x0) / div;
            let b = (-v0 + (beta - zeta * omega_0) * x0) / div;

            let x_t = a * exp1 + b * exp2;
            let v_t = a * r1 * exp1 + b * r2 * exp2;

            (x_t + target, v_t)
        } else {
            // Critically damped (zeta approx 1.0)
            let exp = (-omega_0 * dt).exp();
            let b = v0 + omega_0 * x0;

            // x(t) = exp * (x0 + B * t)
            let x_t = exp * (x0 + b * dt);
            // v(t) = exp * (v0 - omega_0 * B * t)
            let v_t = exp * (v0 - omega_0 * b * dt);

            (x_t + target, v_t)
        };

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
