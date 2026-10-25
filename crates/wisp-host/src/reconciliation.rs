//! Preparation is injectable so desktop transactions can be audited without a display or GPU.
use crate::Config;
use anyhow::Result;

/// Keep renderer state until every changed output has been prepared successfully.
pub fn reconcile<T, D>(
    outputs: &mut Vec<T>,
    config: &mut Config,
    replacement: Option<Config>,
    desired: impl FnOnce(&Config) -> Result<Vec<D>>,
    matches: impl Fn(&T, &D) -> bool,
    mut prepare: impl FnMut(&D) -> Result<T>,
) -> Result<()> {
    enum Prepared<T> {
        Existing(usize),
        Replacement(T),
    }
    let desired = desired(replacement.as_ref().unwrap_or(config))?;
    let mut prepared = Vec::with_capacity(desired.len());
    for target in &desired {
        if let Some(index) = outputs.iter().enumerate().find_map(|(index, output)| {
            let already_used = prepared
                .iter()
                .any(|entry| matches!(entry, Prepared::Existing(existing) if *existing == index));
            (!already_used && matches(output, target)).then_some(index)
        }) {
            prepared.push(Prepared::Existing(index));
        } else {
            prepared.push(Prepared::Replacement(prepare(target)?));
        }
    }
    let mut previous: Vec<_> = std::mem::take(outputs).into_iter().map(Some).collect();
    *outputs = prepared
        .into_iter()
        .map(|output| match output {
            Prepared::Existing(index) => previous[index].take().expect("unique output identities"),
            Prepared::Replacement(output) => output,
        })
        .collect();
    if let Some(replacement) = replacement {
        *config = replacement;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum DesktopAction {
    Retain,
    Release,
    Discover,
}

/// Dead parents invalidate swapchains even when their child handles still exist.
pub fn desktop_action(desktop_valid: Option<bool>, children_valid: bool) -> DesktopAction {
    match desktop_valid {
        None => DesktopAction::Discover,
        Some(false) => DesktopAction::Release,
        Some(true) if !children_valid => DesktopAction::Release,
        Some(true) => DesktopAction::Retain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Assignment;
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq)]
    struct Output {
        name: String,
        assignment: Assignment,
        renderer_identity: u32,
        history: u32,
    }
    fn assignments(package: &str) -> Config {
        Config {
            outputs: ["first", "second"]
                .into_iter()
                .map(|name| (name.into(), Assignment::new(package.into(), 30)))
                .collect(),
            ..Default::default()
        }
    }
    fn apply(
        outputs: &mut Vec<Output>,
        config: &mut Config,
        next: Config,
        fail_second: bool,
    ) -> Result<usize> {
        let mut prepared = 0;
        reconcile(
            outputs,
            config,
            Some(next),
            |config| Ok(config.outputs.clone().into_iter().collect::<Vec<_>>()),
            |output, (name, assignment)| output.name == *name && output.assignment == *assignment,
            |(name, assignment)| {
                prepared += 1;
                anyhow::ensure!(
                    !(fail_second && prepared == 2),
                    "injected second renderer failure"
                );
                Ok(Output {
                    name: name.clone(),
                    assignment: assignment.clone(),
                    renderer_identity: 100 + prepared as u32,
                    history: 0,
                })
            },
        )?;
        Ok(prepared)
    }
    fn original_outputs(config: &Config) -> Vec<Output> {
        config
            .outputs
            .iter()
            .enumerate()
            .map(|(index, (name, assignment))| Output {
                name: name.clone(),
                assignment: assignment.clone(),
                renderer_identity: index as u32,
                history: 42,
            })
            .collect()
    }
    #[test]
    fn second_preparation_failure_preserves_config_and_every_renderer_then_recovers() {
        let mut config = assignments("old.wisp");
        let mut outputs = original_outputs(&config);
        let expected = original_outputs(&config);
        assert!(apply(&mut outputs, &mut config, assignments("new.wisp"), true).is_err());
        assert_eq!(config.outputs, assignments("old.wisp").outputs);
        assert_eq!(outputs, expected);
        assert_eq!(
            apply(
                &mut outputs,
                &mut config,
                assignments("recovered.wisp"),
                false
            )
            .unwrap(),
            2
        );
        assert_eq!(config.outputs, assignments("recovered.wisp").outputs);
        assert!(
            outputs
                .iter()
                .all(|output| output.history == 0 && output.renderer_identity >= 100)
        );
    }
    #[test]
    fn audio_only_and_unrelated_assignment_changes_keep_renderer_history() {
        let mut config = assignments("old.wisp");
        let mut outputs = original_outputs(&config);
        let expected = original_outputs(&config);
        let mut next = config.clone();
        next.audio.enabled = true;
        assert_eq!(apply(&mut outputs, &mut config, next, true).unwrap(), 0);
        assert!(config.audio.enabled);
        assert_eq!(outputs, expected);
        let next = Config {
            outputs: BTreeMap::from([("first".into(), config.outputs["first"].clone())]),
            ..config.clone()
        };
        assert_eq!(apply(&mut outputs, &mut config, next, true).unwrap(), 0);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0], expected[0]);
    }
    #[test]
    fn explorer_loss_releases_surfaces_and_recovery_discovers_a_new_desktop() {
        let mut config = assignments("accepted.wisp");
        let mut outputs = original_outputs(&config);
        assert_eq!(desktop_action(Some(true), true), DesktopAction::Retain);
        assert_eq!(desktop_action(Some(false), true), DesktopAction::Release);
        outputs.clear();
        assert_eq!(desktop_action(Some(true), false), DesktopAction::Release);
        assert_eq!(desktop_action(None, true), DesktopAction::Discover);
        assert_eq!(desktop_action(None, false), DesktopAction::Discover);
        assert!(outputs.is_empty());
        let recovered = config.clone();
        assert_eq!(
            apply(&mut outputs, &mut config, recovered, false).unwrap(),
            2
        );
        assert_eq!(config.outputs, assignments("accepted.wisp").outputs);
        assert!(outputs.iter().all(|output| output.renderer_identity >= 100));
        assert_eq!(desktop_action(Some(true), true), DesktopAction::Retain);
    }
}
