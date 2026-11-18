use super::*;

#[test]
fn test_resume_forward_from_every_checkpoint() {
    let checkpoints = [
        steps::Step::Initialize,
        steps::Step::RenameBranches,
        steps::Step::MoveWorktrees,
        steps::Step::RemoteOps,
        steps::Step::MoveFeatureDir,
        steps::Step::UpdateChildren,
        steps::Step::MoveSessions,
    ];

    for (idx, &step_checkpoint) in checkpoints.iter().enumerate() {
        let old_name_str = format!("fw-old-{idx}");
        let new_name_str = format!("fw-new-{idx}");
        let new_branch_str = format!("feat/fw-new-b-{idx}");

        let tc = setup_test_hall(&old_name_str, &new_name_str, &new_branch_str);
        let git = git::System;
        let plan = build_test_plan(&tc, &git);

        for &s in &checkpoints[..idx] {
            steps::perform_step(&tc.layout, &tc.manifest, &git, &plan, s).unwrap();
        }

        let m_path = steps::marker_path(&plan);
        if let Some(parent) = m_path.parent() {
            fs::ensure_dir(parent).unwrap();
        }
        let transition = steps::Transition {
            version: 1,
            plan: plan.clone(),
            direction: steps::Direction::Forward,
            step: step_checkpoint,
        };
        json::write_canonical(&m_path, &transition).unwrap();

        let report = steps::resume(&tc.layout, &tc.manifest, &git, &m_path, transition).unwrap();
        assert!(report.is_clean());

        assert!(Feature::read(&tc.layout, &tc.old_name).unwrap().is_none());
        let updated = Feature::read(&tc.layout, &tc.new_name).unwrap().unwrap();
        assert_eq!(updated.name, tc.new_name);
        assert_eq!(updated.branch, tc.new_branch);

        for repo_plan in &plan.repos {
            assert!(fs::is_dir(&repo_plan.new_worktree).unwrap());
        }

        let child_name = FeatureName::new("child-feat").unwrap();
        let child = Feature::read(&tc.layout, &child_name).unwrap().unwrap();
        assert_eq!(child.parent, Some(tc.new_name.clone()));

        let sessions = lookup::list_feature(&tc.layout, &tc.new_name).unwrap();
        assert_eq!(sessions.len(), 1);
        let s_ref = &sessions[0];
        assert_eq!(
            s_ref.state.as_ref().unwrap().feature,
            Some(tc.new_name.clone())
        );
        assert!(fs::is_dir(&s_ref.view_dir).unwrap());

        assert!(fs::is_dir(&plan.new_dir).unwrap());
        assert!(!fs::is_dir(&plan.old_dir).unwrap());

        assert!(!fs::is_file(&m_path).unwrap());
        assert!(!fs::is_file(&plan.old_dir.join(".renaming")).unwrap());
        assert!(!fs::is_file(&plan.new_dir.join(".renaming")).unwrap());
    }
}

#[test]
fn test_resume_rollback_from_every_checkpoint() {
    let checkpoints = [
        steps::Step::Initialize,
        steps::Step::RenameBranches,
        steps::Step::MoveWorktrees,
        steps::Step::RemoteOps,
        steps::Step::MoveFeatureDir,
        steps::Step::UpdateChildren,
        steps::Step::MoveSessions,
    ];

    for (idx, &step_checkpoint) in checkpoints.iter().enumerate() {
        let old_name_str = format!("rb-old-{idx}");
        let new_name_str = format!("rb-new-{idx}");
        let new_branch_str = format!("feat/rb-new-b-{idx}");

        let tc = setup_test_hall(&old_name_str, &new_name_str, &new_branch_str);
        let git = git::System;
        let plan = build_test_plan(&tc, &git);

        for &s in &checkpoints[..=idx] {
            steps::perform_step(&tc.layout, &tc.manifest, &git, &plan, s).unwrap();
        }

        let m_path = steps::marker_path(&plan);
        if let Some(parent) = m_path.parent() {
            fs::ensure_dir(parent).unwrap();
        }
        let transition = steps::Transition {
            version: 1,
            plan: plan.clone(),
            direction: steps::Direction::RollingBack,
            step: step_checkpoint,
        };
        json::write_canonical(&m_path, &transition).unwrap();

        let res = steps::resume(&tc.layout, &tc.manifest, &git, &m_path, transition);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.status, Status::Blocked);
        assert_eq!(err.code, "rename.rolled_back");

        assert!(Feature::read(&tc.layout, &tc.new_name).unwrap().is_none());
        let restored = Feature::read(&tc.layout, &tc.old_name).unwrap().unwrap();
        assert_eq!(restored.name, tc.old_name);
        assert_eq!(restored.branch, tc.old_branch);

        for repo_plan in &plan.repos {
            assert!(fs::is_dir(&repo_plan.old_worktree).unwrap());
        }

        let child_name = FeatureName::new("child-feat").unwrap();
        let child = Feature::read(&tc.layout, &child_name).unwrap().unwrap();
        assert_eq!(child.parent, Some(tc.old_name.clone()));

        let sessions = lookup::list_feature(&tc.layout, &tc.old_name).unwrap();
        assert_eq!(sessions.len(), 1);
        let s_ref = &sessions[0];
        assert_eq!(
            s_ref.state.as_ref().unwrap().feature,
            Some(tc.old_name.clone())
        );
        assert!(fs::is_dir(&s_ref.view_dir).unwrap());

        assert!(fs::is_dir(&plan.old_dir).unwrap());
        assert!(!fs::is_dir(&plan.new_dir).unwrap());

        assert!(!fs::is_file(&m_path).unwrap());
        assert!(!fs::is_file(&plan.old_dir.join(".renaming")).unwrap());
        assert!(!fs::is_file(&plan.new_dir.join(".renaming")).unwrap());
    }
}
