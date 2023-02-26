use crate::file_system_interaction::config::GameConfig;
use crate::player_control::actions::CameraAction;
use crate::player_control::camera::util::clamp_pitch;
use crate::player_control::camera::ThirdPersonCamera;
use anyhow::{Context, Result};
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Reflect, FromReflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct FirstPersonCamera {
    pub transform: Transform,
    pub look_target: Option<Vec3>,
    pub up: Vec3,
    pub config: GameConfig,
}

impl Default for FirstPersonCamera {
    fn default() -> Self {
        Self {
            transform: default(),
            look_target: default(),
            up: Vec3::Y,
            config: default(),
        }
    }
}

impl From<&ThirdPersonCamera> for FirstPersonCamera {
    fn from(camera: &ThirdPersonCamera) -> Self {
        let transform = camera.transform.with_translation(camera.target);
        Self {
            transform,
            look_target: camera.secondary_target,
            up: camera.up,
            config: camera.config.clone(),
        }
    }
}

impl FirstPersonCamera {
    pub fn forward(&self) -> Vec3 {
        self.transform.forward()
    }

    pub fn update_transform(
        &mut self,
        dt: f32,
        camera_actions: &ActionState<CameraAction>,
        transform: Transform,
    ) -> Result<Transform> {
        if let Some(look_target) = self.look_target {
            self.look_at(look_target);
        } else {
            let camera_movement = camera_actions
                .axis_pair(CameraAction::Pan)
                .context("Camera movement is not an axis pair")?
                .xy();
            self.handle_camera_controls(camera_movement);
        }
        Ok(self.get_camera_transform(dt, transform))
    }

    fn get_camera_transform(&self, dt: f32, mut transform: Transform) -> Transform {
        let translation_smoothing = self.config.camera.first_person.translation_smoothing;
        let scale = (translation_smoothing * dt).min(1.);
        transform.translation = transform
            .translation
            .lerp(self.transform.translation, scale);

        let rotation_smoothing = self.config.camera.first_person.rotation_smoothing;
        let scale = (rotation_smoothing * dt).min(1.);
        transform.rotation = transform.rotation.slerp(self.transform.rotation, scale);

        transform
    }

    fn handle_camera_controls(&mut self, camera_movement: Vec2) {
        let yaw = -camera_movement.x * self.config.camera.mouse_sensitivity_x;
        let pitch = -camera_movement.y * self.config.camera.mouse_sensitivity_y;
        let pitch = self.clamp_pitch(pitch);
        self.rotate(yaw, pitch);
    }

    fn look_at(&mut self, target: Vec3) {
        let up = self.up;
        self.transform.look_at(target, up);
    }

    fn rotate(&mut self, yaw: f32, pitch: f32) {
        let yaw_rotation = Quat::from_axis_angle(self.up, yaw);
        let pitch_rotation = Quat::from_axis_angle(self.transform.local_x(), pitch);

        let rotation = yaw_rotation * pitch_rotation;
        self.transform.rotate(rotation);
    }

    fn clamp_pitch(&self, angle: f32) -> f32 {
        clamp_pitch(
            self.up,
            self.forward(),
            angle,
            self.config.camera.first_person.most_acute_from_above,
            self.config.camera.first_person.most_acute_from_below,
        )
    }
}
