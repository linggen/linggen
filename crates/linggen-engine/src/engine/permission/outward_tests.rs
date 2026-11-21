//! Outward commands (`permission-spec.md` § Outward commands): a command
//! whose effect leaves this machine asks whatever the session's grants, one
//! answer for that one command; everything else is judged as before.

use super::*;
use std::path::Path;

const WD: &str = "/outward-test/wd";

/// A session with admin on its folder, or on everything.
fn sessions() -> [SessionPermissions; 2] {
    let mut folder = SessionPermissions::default();
    folder.set_path_mode("*", PermissionMode::Read);
    folder.set_path_mode(WD, PermissionMode::Admin);
    let mut everything = SessionPermissions::default();
    everything.set_path_mode("*", PermissionMode::Admin);
    [folder, everything]
}

fn check(cmd: &str, perms: &SessionPermissions) -> PermissionCheckResult {
    check_permission("Bash", Some(cmd), None, Path::new(WD), perms, None)
}

fn asks_once(result: &PermissionCheckResult) -> bool {
    matches!(
        result,
        PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { once: Some(OnceAsk::Outward), .. })
    )
}

const OUTWARD: &[&str] = &[
    // Publishing.
    "cargo publish",
    "cargo publish --dry-run",
    "cargo +nightly publish",
    "npm publish",
    "npm --registry=https://r.example publish",
    "pnpm publish --access public",
    "yarn publish",
    "bun publish",
    "gh release create v1.0.0 dist/x.tar.gz",
    "gh release edit v1.0.0 --draft=false",
    "gh release upload v1.0.0 x.zip",
    "gh release delete v1.0.0 -y",
    "gh pr merge 12 --squash",
    "gh -R owner/repo pr merge 12",
    "flutter pub publish",
    "dart pub publish --force",
    "docker push ghcr.io/x/y:1",
    "podman push x/y",
    "gem push x-1.0.gem",
    "twine upload dist/*",
    "pod trunk push X.podspec",
    // Remote operation.
    "ssh host",
    "ssh -i key user@host uptime",
    "scp a.txt host:/tmp/",
    "sftp host",
    "rsync -a dist/ host:/srv/www/",
    "rsync -az dist/ user@host:www",
    "rsync -a rsync://host/module/ local/",
    "aws s3 ls",
    "aws ec2 describe-instances",
    "gcloud app deploy",
    "az group list",
    "kubectl get pods",
    "kubectl apply -f k.yaml",
    "helm upgrade x ./chart",
    "terraform apply",
    "pulumi up",
    "wrangler deploy",
    "vercel --prod",
    "netlify deploy",
    "fly deploy",
    "flyctl deploy",
    "heroku releases",
    "firebase deploy",
    // A push that is more than a plain push.
    "git push origin main",
    "git push origin master",
    "git push origin HEAD:main",
    "git push origin feature:master",
    "git push origin HEAD:refs/heads/main",
    "git push -f",
    "git push --force origin feature",
    "git push -uf origin feature",
    "git push --force-with-lease",
    "git push --force-with-lease=feature:abc origin feature",
    "git push origin +feature",
    "git push --delete origin feature",
    "git push origin -d feature",
    "git push origin :feature",
    "git push --mirror",
    "git push --all",
    "git -C sub push -f",
    "git -c push.default=current push origin main",
    // In a compound command.
    "cargo test && cargo publish",
    "git add . && git commit -m x && git push origin main",
];

#[test]
fn every_outward_command_asks_whatever_the_grants() {
    for perms in sessions() {
        for cmd in OUTWARD {
            assert!(asks_once(&check(cmd, &perms)), "{cmd}");
        }
    }
}

#[test]
fn an_outward_program_asks_however_it_is_named() {
    for perms in sessions() {
        for cmd in [
            "/usr/bin/ssh host",
            "/opt/homebrew/bin/gh release create v1",
            "FOO=1 cargo publish",
            "env cargo publish",
            "env -i FOO=1 npm publish",
            "nice -n 19 git push --force",
            "/usr/bin/env git push origin main",
        ] {
            assert!(asks_once(&check(cmd, &perms)), "{cmd}");
        }
    }
}

#[test]
fn a_plain_push_and_local_work_do_not_ask() {
    for perms in sessions() {
        for cmd in [
            "git push",
            "git push origin feature",
            "git push -u origin feature",
            "git push --set-upstream origin feature",
            "git push --tags",
            "git push origin feature:feature",
            "gh pr view 12",
            "gh release list",
            "git fetch",
            "git pull",
            "npm install",
            "cargo build",
            "cargo test publish_flow",
            "rsync -a dist/ ./out/",
            "docker images",
        ] {
            assert!(
                matches!(check(cmd, &perms), PermissionCheckResult::Allowed),
                "{cmd}"
            );
        }
    }
}

#[test]
fn a_command_that_only_names_an_outward_program_does_not_ask() {
    for perms in sessions() {
        for cmd in [
            "echo ssh",
            "man scp",
            "grep publish notes.txt",
            "git commit -m \"publish notes\"",
            "git commit -m 'git push --force'",
            "which kubectl",
            "command -v aws",
            "cat .github/workflows/publish.yml",
        ] {
            assert!(
                matches!(check(cmd, &perms), PermissionCheckResult::Allowed),
                "{cmd}"
            );
        }
    }
}

#[test]
fn the_deny_floor_still_blocks_first() {
    for perms in sessions() {
        let result = check("cargo publish && sudo ls", &perms);
        assert!(
            matches!(result, PermissionCheckResult::Blocked(_)),
            "{result:?}"
        );
    }
}

#[test]
fn an_outward_command_beyond_the_grants_is_still_one_ask_for_that_command() {
    // Edit on the folder only: the path outside is part of the same ask.
    let mut perms = SessionPermissions::default();
    perms.set_path_mode(WD, PermissionMode::Edit);
    let result = check("scp /outside/secret host:", &perms);
    assert!(asks_once(&result), "{result:?}");
    let PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
        path, directories, ..
    }) = result
    else {
        unreachable!()
    };
    assert_eq!(path, "/outside/secret");
    assert!(directories.is_empty(), "no folder grant for an outward ask");
}

#[test]
fn an_outward_question_offers_once_or_deny_only() {
    let q = build_outward_question("Bash cargo publish", &PermissionMode::Admin, WD);
    let labels: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, [ALLOW_ONCE, "Deny"]);
    assert!(q.permission.is_some(), "parsed against its offered options");
    let offered: Vec<String> = labels.iter().map(|l| l.to_string()).collect();
    let answer = |label: &str| crate::engine::tools::AskUserAnswer {
        question_index: 0,
        selected: vec![label.to_string()],
        custom_text: None,
    };
    assert_eq!(
        parse_exceeds_ceiling_answer(&[answer(ALLOW_ONCE)], &PermissionMode::Admin, &offered),
        PermissionAction::AllowOnce
    );
    // A grant that was not offered denies.
    assert_eq!(
        parse_exceeds_ceiling_answer(
            &[answer("grant:all:admin")],
            &PermissionMode::Admin,
            &offered
        ),
        PermissionAction::Deny
    );
}
