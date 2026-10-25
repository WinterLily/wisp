use anyhow::{Context, Result, ensure};
use wisp_host::{Assignment, Config};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Monitor {
    /// Monitor device interface path, falling back to the GDI name when unavailable.
    pub id: String,
    pub name: String,
    pub description: String,
    pub rect: Bounds,
    pub primary: bool,
    pub dpi: (u32, u32),
}
impl Monitor {
    pub fn assignment<'a>(&self, config: &'a Config) -> Option<&'a Assignment> {
        if config.disabled_outputs.contains(&self.id)
            || config.disabled_outputs.contains(&self.name)
        {
            return None;
        }
        config
            .outputs
            .get(&self.id)
            .or_else(|| config.outputs.get(&self.name))
            .or(config.default.as_ref())
    }
}

/// Physical virtual-desktop coordinates. Monitors left/above the primary are negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Bounds {
    pub fn size(self, limit: u32) -> Result<(u32, u32)> {
        let width = self
            .right
            .checked_sub(self.left)
            .context("monitor width overflow")?;
        let height = self
            .bottom
            .checked_sub(self.top)
            .context("monitor height overflow")?;
        ensure!(width > 0 && height > 0, "empty monitor bounds");
        ensure!(
            width as u32 <= limit && height as u32 <= limit,
            "monitor dimensions exceed GPU limits"
        );
        Ok((width as u32, height as u32))
    }
    pub fn mouse(self, x: i32, y: i32) -> [f32; 2] {
        [
            ((f64::from(x) - f64::from(self.left)) / (f64::from(self.right) - f64::from(self.left)))
                .clamp(0.0, 1.0) as f32,
            ((f64::from(y) - f64::from(self.top)) / (f64::from(self.bottom) - f64::from(self.top)))
                .clamp(0.0, 1.0) as f32,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negative_origins_portrait_and_cursor_coordinates() {
        let rect = Bounds {
            left: -1080,
            top: -200,
            right: 0,
            bottom: 1720,
        };
        assert_eq!(rect.size(8192).unwrap(), (1080, 1920));
        assert_eq!(rect.mouse(-540, 760), [0.5, 0.5]);
        assert_eq!(rect.mouse(200, -300), [1.0, 0.0]);
        assert!(rect.size(1024).is_err());
        assert!(
            Bounds {
                right: -1080,
                ..rect
            }
            .size(8192)
            .is_err()
        );
        assert!(
            Bounds {
                left: i32::MIN,
                right: i32::MAX,
                ..rect
            }
            .size(u32::MAX)
            .is_err()
        );
    }
    #[test]
    fn stable_id_wins_over_gdi_alias_and_default() {
        let monitor = Monitor {
            id: "interface-id".into(),
            name: r"\\.\DISPLAY1".into(),
            description: "panel".into(),
            rect: Bounds {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            primary: true,
            dpi: (96, 96),
        };
        let mut config = Config {
            default: Some(Assignment::new("default.wisp".into(), 30)),
            ..Default::default()
        };
        assert_eq!(monitor.assignment(&config).unwrap().fps, 30);
        config.outputs.insert(
            monitor.name.clone(),
            Assignment::new("alias.wisp".into(), 60),
        );
        assert_eq!(monitor.assignment(&config).unwrap().fps, 60);
        config.outputs.insert(
            monitor.id.clone(),
            Assignment::new("stable.wisp".into(), 24),
        );
        assert_eq!(monitor.assignment(&config).unwrap().fps, 24);
        config.disabled_outputs.insert(monitor.id.clone());
        assert!(monitor.assignment(&config).is_none());
        config.disabled_outputs.clear();
        config.disabled_outputs.insert(monitor.name.clone());
        assert!(monitor.assignment(&config).is_none());
        config.disabled_outputs.clear();
        config.default = None;
        config.outputs.clear();
        assert!(monitor.assignment(&config).is_none());
    }
}
