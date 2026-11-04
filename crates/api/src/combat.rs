//! Hitting things: targets that take hits, and the melee tools that deal them. The engine owns
//! the bodies, the swing and the hit rules; the mod supplies the numbers.

use crate::{Tunable, Tunables};

/// A part of a target a strike can hit, as a box in its model's space: meters, +Z up, feet at
/// the origin, front facing +Y. It turns with the target's facing.
#[derive(Clone, Debug, PartialEq)]
pub struct HitBox {
    /// Such as `"head"`, for telling hits apart.
    pub name: String,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// What a hit here multiplies the tool's damage by.
    pub damage: f32,
}

/// Something that can be hit, knocked around and broken: a body with health, and a will of its
/// own if it names a brain. Placed in the world with [`crate::WorldEdit::add_target`], and back
/// at its spot a while after breaking.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetDef {
    /// Namespaced, such as `"base:cow"`.
    pub name: String,
    /// The registered model it's drawn as.
    pub model: String,
    /// A registered sound it makes when hit, if any.
    pub hit_sound: Option<String>,
    /// The registered [`crate::Brain`] that decides where it walks and what it plays. `None`
    /// is a dummy that only moves when hit.
    pub brain: Option<String>,
    /// Width and depth of its collision box, in meters.
    pub width: f32,
    /// Height of its collision box, from the feet.
    pub height: f32,
    /// How high a ledge it walks up without jumping, in meters.
    pub step_height: f32,
    /// The parts strikes can hit. Empty uses the collision box, at full damage. Terrain only
    /// ever meets the collision box.
    pub hit_boxes: Vec<HitBox>,
    /// Damage it takes before it breaks.
    pub health: f32,
    /// How hard it is to knock around: a hit's knockback is divided by it.
    pub weight: f32,
    /// Downward acceleration, meters per second squared.
    pub gravity: f32,
    /// How fast it slows down sliding along the ground, per second, on ground of friction 1.
    /// A creature walking eases toward the speed its brain wants at the same rate.
    pub drag: f32,
    /// How fast it slows down in the air, per second, so a hit sends it off fast and it stops
    /// short instead of sailing.
    pub air_drag: f32,
    /// Share of its speed it keeps when it hits something: 0 stops dead, 1 bounces forever.
    pub bounce: f32,
    /// Seconds from breaking to being back at its spot.
    pub respawn_time: f32,
    /// Whether it comes back on its own after `respawn_time`. Without, a broken one stays gone
    /// until every creature is put back, such as when the player is knocked out (see
    /// [`crate::HealthDef::knock_out_resets`]).
    pub respawns: bool,
    /// Sideways speed into a wall, meters per second, from which hitting it hurts: a slam.
    pub slam_speed: f32,
    /// Damage a slam deals for each meter per second over `slam_speed`.
    pub slam_damage: f32,
}

impl TargetDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        let positive = [("width", self.width), ("height", self.height), ("health", self.health), ("weight", self.weight)];
        if let Some((name, value)) = positive.iter().find(|(_, v)| !(v.is_finite() && *v > 0.0)) {
            return Err(format!("{name} must be positive, got {value}"));
        }
        let at_least_zero = [
            ("gravity", self.gravity),
            ("drag", self.drag),
            ("air_drag", self.air_drag),
            ("respawn_time", self.respawn_time),
            ("slam_speed", self.slam_speed),
            ("slam_damage", self.slam_damage),
        ];
        if let Some((name, value)) = at_least_zero.iter().find(|(_, v)| !(v.is_finite() && *v >= 0.0)) {
            return Err(format!("{name} must be at least 0, got {value}"));
        }
        if !(self.step_height >= 0.0 && self.step_height < self.height) {
            return Err(format!("step_height must be at least 0 and below the height, got {}", self.step_height));
        }
        if !(0.0..=1.0).contains(&self.bounce) {
            return Err(format!("bounce must be from 0 to 1, got {}", self.bounce));
        }
        for b in &self.hit_boxes {
            let sized = (0..3).all(|i| b.min[i].is_finite() && b.max[i].is_finite() && b.min[i] < b.max[i]);
            if !sized {
                return Err(format!("hit box {:?} needs min below max on every axis", b.name));
            }
            if !(b.damage.is_finite() && b.damage >= 0.0) {
                return Err(format!("hit box {:?} damage must be at least 0, got {}", b.name, b.damage));
            }
        }
        Ok(())
    }
}

impl Tunables for TargetDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("health", self.health, 1.0, 200.0),
            Tunable::new("step_height", self.step_height, 0.0, 1.2),
            Tunable::new("weight", self.weight, 0.1, 20.0),
            Tunable::new("gravity", self.gravity, 0.0, 60.0),
            Tunable::new("drag", self.drag, 0.0, 30.0),
            Tunable::new("air_drag", self.air_drag, 0.0, 15.0),
            Tunable::new("bounce", self.bounce, 0.0, 1.0),
            Tunable::new("respawn_time", self.respawn_time, 0.0, 30.0),
            Tunable::new("slam_speed", self.slam_speed, 0.0, 20.0),
            Tunable::new("slam_damage", self.slam_damage, 0.0, 10.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "health" => &mut self.health,
            "step_height" => &mut self.step_height,
            "weight" => &mut self.weight,
            "gravity" => &mut self.gravity,
            "drag" => &mut self.drag,
            "air_drag" => &mut self.air_drag,
            "bounce" => &mut self.bounce,
            "respawn_time" => &mut self.respawn_time,
            "slam_speed" => &mut self.slam_speed,
            "slam_damage" => &mut self.slam_damage,
            _ => return false,
        };
        *field = value;
        true
    }
}

/// A melee tool. Held, its main use swings it: a wind-up, then the strike along where the
/// player looks, then a recovery before the next swing. Its second use raises a guard.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
    /// Namespaced, such as `"base:club"`.
    pub name: String,
    /// How far from the eyes the strike lands, in meters.
    pub reach: f32,
    /// How far off the line of sight a target can be and still get hit, in meters, so a
    /// near miss still connects.
    pub radius: f32,
    /// Damage per hit.
    pub damage: f32,
    /// Speed the strike knocks a target of weight 1 away at, level along the swing, in meters
    /// per second.
    pub knockback: f32,
    /// Upward speed the strike adds for a target of weight 1, in meters per second.
    pub lift: f32,
    /// Share of the player's own speed toward the target that the strike passes on to it, as
    /// extra knockback, whatever its weight: sprinting or falling into a swing hits further.
    pub momentum: f32,
    /// Extra damage for each meter per second the player moves toward the target.
    pub momentum_damage: f32,
    /// Seconds from starting a swing to the strike.
    pub windup: f32,
    /// Seconds after the strike before the next swing can start.
    pub recover: f32,
}

impl ToolDef {
    /// Checks the values make sense.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.reach.is_finite() && self.reach > 0.0) {
            return Err(format!("reach must be positive, got {}", self.reach));
        }
        let fields = [
            ("radius", self.radius),
            ("damage", self.damage),
            ("knockback", self.knockback),
            ("lift", self.lift),
            ("momentum", self.momentum),
            ("momentum_damage", self.momentum_damage),
            ("windup", self.windup),
            ("recover", self.recover),
        ];
        if let Some((name, value)) = fields.iter().find(|(_, v)| !(v.is_finite() && *v >= 0.0)) {
            return Err(format!("{name} must be at least 0, got {value}"));
        }
        Ok(())
    }
}

impl Tunables for ToolDef {
    fn tunables(&self) -> Vec<Tunable> {
        vec![
            Tunable::new("reach", self.reach, 0.5, 8.0),
            Tunable::new("radius", self.radius, 0.0, 1.5),
            Tunable::new("damage", self.damage, 0.0, 50.0),
            Tunable::new("knockback", self.knockback, 0.0, 40.0),
            Tunable::new("lift", self.lift, 0.0, 20.0),
            Tunable::new("momentum", self.momentum, 0.0, 2.0),
            Tunable::new("momentum_damage", self.momentum_damage, 0.0, 5.0),
            Tunable::new("windup", self.windup, 0.0, 1.0),
            Tunable::new("recover", self.recover, 0.0, 2.0),
        ]
    }

    fn set_tunable(&mut self, name: &str, value: f32) -> bool {
        let field = match name {
            "reach" => &mut self.reach,
            "radius" => &mut self.radius,
            "damage" => &mut self.damage,
            "knockback" => &mut self.knockback,
            "lift" => &mut self.lift,
            "momentum" => &mut self.momentum,
            "momentum_damage" => &mut self.momentum_damage,
            "windup" => &mut self.windup,
            "recover" => &mut self.recover,
            _ => return false,
        };
        *field = value;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> TargetDef {
        TargetDef {
            name: "test:dummy".into(),
            model: "test:dummy".into(),
            hit_sound: None,
            brain: None,
            width: 0.9,
            height: 1.3,
            step_height: 0.5,
            hit_boxes: vec![HitBox { name: "head".into(), min: [-0.2, 0.4, 1.0], max: [0.2, 0.8, 1.4], damage: 2.0 }],
            health: 20.0,
            weight: 3.0,
            gravity: 25.0,
            drag: 4.0,
            air_drag: 2.0,
            bounce: 0.2,
            respawn_time: 3.0,
            slam_speed: 4.0,
            slam_damage: 1.0,
            respawns: true,
        }
    }

    fn tool() -> ToolDef {
        ToolDef {
            name: "test:club".into(),
            reach: 2.5,
            radius: 0.3,
            damage: 5.0,
            knockback: 10.0,
            lift: 4.0,
            momentum: 0.8,
            momentum_damage: 0.5,
            windup: 0.08,
            recover: 0.3,
        }
    }

    #[test]
    fn checks_its_values() {
        assert_eq!(target().validate(), Ok(()));
        assert!(TargetDef { health: 0.0, ..target() }.validate().is_err());
        assert!(TargetDef { weight: f32::NAN, ..target() }.validate().is_err());
        assert!(TargetDef { bounce: 2.0, ..target() }.validate().is_err());
        assert!(TargetDef { step_height: 1.3, ..target() }.validate().is_err(), "as high as itself");
        let flat = HitBox { name: "flat".into(), min: [0.0; 3], max: [1.0, 1.0, 0.0], damage: 1.0 };
        assert!(TargetDef { hit_boxes: vec![flat], ..target() }.validate().is_err());
        assert_eq!(tool().validate(), Ok(()));
        assert!(ToolDef { reach: 0.0, ..tool() }.validate().is_err());
        assert!(ToolDef { windup: -1.0, ..tool() }.validate().is_err());
    }

    #[test]
    fn every_value_is_tunable() {
        let mut t = target();
        for v in t.tunables() {
            assert!(t.set_tunable(&v.name, v.min), "{} sets", v.name);
        }
        let mut c = tool();
        for v in c.tunables() {
            assert!(c.set_tunable(&v.name, v.min), "{} sets", v.name);
        }
        assert!(!t.set_tunable("missing", 1.0) && !c.set_tunable("missing", 1.0));
    }
}
