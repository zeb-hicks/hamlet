use crate::file_system_interaction::config::GameConfig;
use crate::player_control::actions::CameraAction;
use crate::player_control::camera::util::clamp_pitch;
use crate::player_control::camera::{FirstPersonCamera, FixedAngleCamera};
use crate::util::trait_extension::{Vec2Ext, Vec3Ext};
use anyhow::{Context, Result};
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Reflect, FromReflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ThirdPersonCamera {
    pub transform: Transform,
    pub target: Vec3,
    pub up: Vec3,
    pub secondary_target: Option<Vec3>,
    pub distance: f32,
    pub config: GameConfig,
}

impl Default for ThirdPersonCamera {
    fn default() -> Self {
        Self {
            up: Vec3::Y,
            transform: default(),
            distance: 5.,
            target: default(),
            secondary_target: default(),
            config: default(),
        }
    }
}

impl From<&FirstPersonCamera> for ThirdPersonCamera {
    fn from(first_person_camera: &FirstPersonCamera) -> Self {
        let target = first_person_camera.transform.translation;
        let distance = first_person_camera.config.camera.third_person.min_distance;
        let eye = target - first_person_camera.forward() * distance;
        let up = first_person_camera.up;
        let eye = Transform::from_translation(eye).looking_at(target, up);
        Self {
            transform: eye,
            target,
            up,
            distance,
            secondary_target: first_person_camera.look_target,
            config: first_person_camera.config.clone(),
        }
    }
}

impl From<&FixedAngleCamera> for ThirdPersonCamera {
    fn from(fixed_angle_camera: &FixedAngleCamera) -> Self {
        let mut transform = fixed_angle_camera.transform;
        let config = fixed_angle_camera.config.clone();
        transform.rotate_axis(
            transform.right(),
            config.camera.third_person.most_acute_from_above,
        );
        Self {
            transform,
            target: fixed_angle_camera.target,
            up: fixed_angle_camera.up,
            distance: fixed_angle_camera.distance,
            secondary_target: fixed_angle_camera.secondary_target,
            config: fixed_angle_camera.config.clone(),
        }
    }
}

impl ThirdPersonCamera {
    pub fn forward(&self) -> Vec3 {
        self.transform.forward()
    }

    fn rotate_around_target(&mut self, yaw: f32, pitch: f32) {
        let yaw_rotation = Quat::from_axis_angle(self.up, yaw);
        let pitch_rotation = Quat::from_axis_angle(self.transform.local_x(), pitch);

        let pivot = self.target;
        let rotation = yaw_rotation * pitch_rotation;
        self.transform.rotate_around(pivot, rotation);
    }

    pub fn update_transform(
        &mut self,
        dt: f32,
        camera_actions: &ActionState<CameraAction>,
        rapier_context: &RapierContext,
        transform: Transform,
    ) -> Result<Transform> {
        if let Some(secondary_target) = self.secondary_target {
            self.move_eye_to_align_target_with(secondary_target);
        }

        let camera_movement = camera_actions
            .axis_pair(CameraAction::Pan)
            .context("Camera movement is not an axis pair")?
            .xy();
        if !camera_movement.is_approx_zero() {
            self.handle_camera_controls(camera_movement);
        }

        let zoom = camera_actions.clamped_value(CameraAction::Zoom);
        self.zoom(zoom);
        let los_correction = self.place_eye_in_valid_position(rapier_context);
        Ok(self.get_camera_transform(dt, transform, los_correction))
    }

    fn handle_camera_controls(&mut self, camera_movement: Vec2) {
        let yaw = -camera_movement.x * self.config.camera.mouse_sensitivity_x;
        let pitch = -camera_movement.y * self.config.camera.mouse_sensitivity_y;
        let pitch = self.clamp_pitch(pitch);
        self.rotate_around_target(yaw, pitch);
    }

    fn clamp_pitch(&self, angle: f32) -> f32 {
        clamp_pitch(
            self.up,
            self.forward(),
            angle,
            self.config.camera.third_person.most_acute_from_above,
            self.config.camera.third_person.most_acute_from_below,
        )
    }

    fn zoom(&mut self, zoom: f32) {
        let zoom_speed = self.config.camera.third_person.zoom_speed;
        let zoom = zoom * zoom_speed;
        let min_distance = self.config.camera.third_person.min_distance;
        let max_distance = self.config.camera.third_person.max_distance;
        self.distance = (self.distance - zoom).clamp(min_distance, max_distance);
    }

    fn move_eye_to_align_target_with(&mut self, secondary_target: Vec3) {
        let target_to_secondary_target = (secondary_target - self.target).split(self.up).horizontal;
        if target_to_secondary_target.is_approx_zero() {
            return;
        }
        let target_to_secondary_target = target_to_secondary_target.normalize();
        let eye_to_target = (self.target - self.transform.translation)
            .split(self.up)
            .horizontal
            .normalize();
        let rotation = Quat::from_rotation_arc(eye_to_target, target_to_secondary_target);
        let pivot = self.target;
        self.transform.rotate_around(pivot, rotation);
    }

    fn place_eye_in_valid_position(
        &mut self,
        rapier_context: &RapierContext,
    ) -> LineOfSightCorrection {
        let line_of_sight_result = self.keep_line_of_sight(rapier_context);
        self.transform.translation = line_of_sight_result.location;
        line_of_sight_result.correction
    }

    fn get_camera_transform(
        &self,
        dt: f32,
        mut transform: Transform,
        line_of_sight_correction: LineOfSightCorrection,
    ) -> Transform {
        let translation_smoothing = if line_of_sight_correction == LineOfSightCorrection::Further {
            self.config
                .camera
                .third_person
                .translation_smoothing_going_further
        } else {
            self.config
                .camera
                .third_person
                .translation_smoothing_going_closer
        };

        let scale = (translation_smoothing * dt).min(1.);
        transform.translation = transform
            .translation
            .lerp(self.transform.translation, scale);

        let rotation_smoothing = self.config.camera.first_person.rotation_smoothing;
        let scale = (rotation_smoothing * dt).min(1.);
        transform.rotation = transform.rotation.slerp(self.transform.rotation, scale);

        transform
    }

    pub fn keep_line_of_sight(&self, rapier_context: &RapierContext) -> LineOfSightResult {
        let origin = self.target;
        let direction = -self.forward();

        let distance = self.get_raycast_distance(origin, direction, rapier_context);
        let location = origin + direction * distance;

        let original_distance = self.target - self.transform.translation;
        let correction = if distance * distance < original_distance.length_squared() - 1e-3 {
            LineOfSightCorrection::Closer
        } else {
            LineOfSightCorrection::Further
        };
        LineOfSightResult {
            location,
            correction,
        }
    }

    pub fn get_raycast_distance(
        &self,
        origin: Vec3,
        direction: Vec3,
        rapier_context: &RapierContext,
    ) -> f32 {
        let max_toi = self.distance;
        let solid = true;
        let mut filter = QueryFilter::only_fixed();
        filter.flags |= QueryFilterFlags::EXCLUDE_SENSORS;

        let min_distance_to_objects = self.config.camera.third_person.min_distance_to_objects;
        rapier_context
            .cast_ray(origin, direction, max_toi, solid, filter)
            .map(|(_entity, toi)| toi - min_distance_to_objects)
            .unwrap_or(max_toi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineOfSightResult {
    pub location: Vec3,
    pub correction: LineOfSightCorrection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOfSightCorrection {
    Closer,
    Further,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn facing_secondary_target_that_is_primary_changes_nothing() {
        let camera_translation = Vec3::new(2., 0., 0.);
        let primary_target = Vec3::new(-2., 0., 0.);
        let secondary_target = Vec3::new(-2., 0., 0.);

        let mut camera = build_camera(camera_translation, primary_target);
        camera.move_eye_to_align_target_with(secondary_target);

        assert_nearly_eq(camera.transform.translation, camera_translation);
    }

    #[test]
    fn facing_secondary_target_that_is_aligned_with_primary_changes_nothing() {
        let camera_translation = Vec3::new(2., 0., 0.);
        let primary_target = Vec3::new(-2., 0., 0.);
        let secondary_target = Vec3::new(-3., 0., 0.);

        let mut camera = build_camera(camera_translation, primary_target);
        camera.move_eye_to_align_target_with(secondary_target);

        assert_nearly_eq(camera.transform.translation, camera_translation);
    }

    #[test]
    fn faces_secondary_target_that_is_at_right_angle_with_primary() {
        let camera_translation = Vec3::new(2., 0., 0.);
        let primary_target = Vec3::new(-2., 0., 0.);
        let secondary_target = Vec3::new(-2., 0., -2.);

        let mut camera = build_camera(camera_translation, primary_target);
        camera.move_eye_to_align_target_with(secondary_target);

        let expected_position = Vec3::new(-2., 0., 4.);
        assert_nearly_eq(camera.transform.translation, expected_position);
    }

    #[test]
    fn faces_secondary_target_that_is_at_right_angle_with_primary_ignoring_y() {
        let camera_translation = Vec3::new(2., 2., 0.);
        let primary_target = Vec3::new(-2., -3., 0.);
        let secondary_target = Vec3::new(-2., -1., -2.);

        let mut camera = build_camera(camera_translation, primary_target);
        camera.move_eye_to_align_target_with(secondary_target);

        let expected_position = Vec3::new(-2., 2., 4.);
        assert_nearly_eq(camera.transform.translation, expected_position);
    }

    fn build_camera(camera_translation: Vec3, primary_target: Vec3) -> ThirdPersonCamera {
        let mut camera = ThirdPersonCamera::default();
        let camera_transform = Transform::from_translation(camera_translation);

        camera.transform = camera_transform.looking_at(primary_target, Vec3::Y);
        camera.target = primary_target;
        camera.distance = camera.target.distance(camera.transform.translation);

        camera
    }

    fn assert_nearly_eq(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length_squared() < 1e-5,
            "expected: {:?}, actual: {:?}",
            expected,
            actual
        );
    }
}
