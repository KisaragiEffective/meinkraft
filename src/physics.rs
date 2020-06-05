use std::time;

use nalgebra_glm::vec3;

use crate::PLAYER_HALF_WIDTH;
use crate::chunk_manager::ChunkManager;
use crate::constants::GRAVITY;
use crate::input::InputCache;
use crate::player::{PlayerPhysicsState, PlayerState};
use std::time::Instant;
use num_traits::Zero;

/// Fixed timestep physics simulation using the following method:
/// https://gafferongames.com/post/fix_your_timestep/
/// With this method, the physics are always deterministic and work independently
/// of the performance of the game

pub trait Interpolatable {
    fn interpolate(&self, alpha: f32, other: &Self) -> Self;
}

impl Interpolatable for f32 {
    fn interpolate(&self, alpha: f32, other: &Self) -> Self {
        self * alpha + (1.0 - alpha) * other
    }
}

pub struct Interpolator<T: Clone + Interpolatable> {
    pub t: f32,
    pub dt: f32,
    pub current_time: time::Instant,
    pub accumulator: f32,
    pub previous_state: T,
    pub current_state: T,
    pub interpolated_state: T,
}

impl <T: Default + Clone + Interpolatable> Default for Interpolator<T> {
    fn default() -> Self {
        Self::new(0., T::default())
    }
}

impl<T: Clone + Interpolatable> Interpolator<T> {
    pub fn new(dt: f32, initial_state: T) -> Self {
        Self {
            t: 0.0,
            dt,
            current_time: time::Instant::now(),
            accumulator: 0.0,
            previous_state: initial_state.clone(),
            current_state: initial_state.clone(),
            interpolated_state: initial_state
        }
    }

    pub fn get_latest_state(&self) -> &T {
        &self.current_state
    }

    pub fn get_latest_state_mut(&mut self) -> &mut T {
        &mut self.current_state
    }

    pub fn get_interpolated_state(&self) -> &T {
        &self.interpolated_state
    }

    /// Advances the physics for a given state.
    pub fn step(&mut self, time: Instant, integrate: &mut dyn FnMut(&T, f32, f32) -> T) {
        let now = time;
        let mut frame_time = now.saturating_duration_since(self.current_time).as_secs_f32();
        if frame_time > 0.25 {
            frame_time = 0.25;
        }
        self.current_time = now;
        self.accumulator += frame_time;

        while self.accumulator >= self.dt {
            self.previous_state = self.current_state.clone();
            self.current_state = integrate(&self.previous_state, self.t, self.dt);
            self.t += self.dt;
            self.accumulator -= self.dt;
        }

        let alpha = self.accumulator / self.dt;
        self.interpolated_state = self.current_state.interpolate(alpha, &self.previous_state);
    }
}

// impl Interpolator<PlayerPhysicsState> {
//     /// Advances the physics for the player.
//     pub fn update_player_physics(&mut self, time: Instant, input_cache: &InputCache, chunk_manager: &ChunkManager, player_properties: &mut PlayerState) {
//         self.step(time, &mut |player: &PlayerPhysicsState, _t: f32, dt: f32| {
//             let mut player = player.clone();
//             if !player_properties.is_flying {
//                 player.acceleration.y += GRAVITY;
//             }
//
//             player.apply_keyboard_mouvement(player_properties, &input_cache);
//             player.velocity += player.acceleration * dt;
//             player.apply_friction(dt, player_properties.is_flying);
//             player.limit_velocity(&player_properties);
//
//             let is_on_ground = |player: &PlayerPhysicsState| {
//                 let mut player = player.clone();
//                 let vy = vec3(0.0, player.velocity.y, 0.0);
//                 player.aabb.ip_translate(&(vy * dt));
//                 let colliding_block = player.get_colliding_block_coords(&chunk_manager);
//                 if let Some(colliding_block) = colliding_block {
//                     player.separate_from_block(&vy, &colliding_block)
//                 } else {
//                     false
//                 }
//             };
//
//             // We are using the Separated Axis Theorem
//             // We decompose the velocity vector into 3 vectors for each dimension
//             // For each one, we move the entity and do the collision detection/resolution
//             let mut is_player_on_ground = false;
//             let separated_axis = &[
//                 vec3(player.velocity.x, 0.0, 0.0),
//                 vec3(0.0, 0.0, player.velocity.z),
//                 vec3(0.0, player.velocity.y, 0.0)];
//
//             for v in separated_axis {
//                 let bk = player.clone();
//                 player.aabb.ip_translate(&(v * dt));
//                 let colliding_block = player.get_colliding_block_coords(&chunk_manager);
//
//                 // Collision resolution
//                 if let Some(colliding_block) = colliding_block {
//                     is_player_on_ground |= player.separate_from_block(&v, &colliding_block);
//                 }
//
//                 if input_cache.is_key_pressed(glfw::Key::LeftShift)
//                     && player.is_on_ground
//                     && !is_on_ground(&player)
//                     && player.velocity.y < 0. {
//                     player = bk;
//
//                     if !v.x.is_zero() {
//                         player.velocity.x = 0.0;
//                     }
//                     if !v.z.is_zero() {
//                         player.velocity.z = 0.0;
//                     }
//                 }
//             }
//             player.is_on_ground = is_player_on_ground;
//             if player.is_on_ground {
//                 player_properties.is_flying = false;
//             }
//
//             // Update the position of the player and reset the acceleration
//             player.position.x = player.aabb.mins.x + PLAYER_HALF_WIDTH;
//             player.position.y = player.aabb.mins.y;
//             player.position.z = player.aabb.mins.z + PLAYER_HALF_WIDTH;
//
//             player.acceleration.x = 0.0;
//             player.acceleration.y = 0.0;
//             player.acceleration.z = 0.0;
//             player
//         });
//     }
// }

impl Interpolator<f32> {
    pub fn interpolate_fov(&mut self, time: Instant, target_fov: f32) {
        self.step(time, &mut |&fov, _t, dt| {
            let convergence = 10.0;
            convergence * dt * target_fov + (1.0 - convergence * dt) * fov
        });
    }
}

impl Interpolator<f32> {
    pub fn interpolate_camera_height(&mut self, time: Instant, target_camera_height: f32) {
        self.step(time, &mut |&camera_height, _t, dt| {
            let convergence = 20.0;
            convergence * dt * target_camera_height + (1.0 - convergence * dt) * camera_height
        });
    }
}