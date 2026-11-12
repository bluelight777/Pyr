# Working on Pyr: read this first

## Never lose the work again

On November 10, 2026 the cloud workspace was wiped. It took the source of Pyr 0.13.1 to 0.13.28
and the release key with it: twelve days of Blue's work, because after 0.13.0 nothing had been
copied out of the workspace. A workspace can be wiped at any moment, without warning. So nothing
lives only in it, ever:

1. **First thing in every session, and in every new copy of the repository** (a restored
   workspace too): `.githooks/install`. It turns the hooks on (`core.hooksPath`) and starts the
   20-minute backups of uncommitted work. Check with `tail /tmp/pyr-backup.log`.
2. **After every commit** the post-commit hook writes the whole repository as a bundle to the
   session's outputs folder (`/mnt/user-data/outputs/pyr-source/pyr-rebuild.bundle`), which came
   through the wipe. Uncommitted work goes beside it every 20 minutes (`pyr-wip.tar.gz`).
3. **Commit often**: at least every hour of work, finished or not, and always before a long
   build or test. Everything on main; never other branches.
4. **After every piece of work, the bundle goes to Blue's PC**: SendUserFile the bundle, then
   device_commit_files it to `Downloads\pyr-source\pyr-rebuild.bundle` (force). His PC pushes it
   to GitHub (`bluelight777/Pyr`) every hour (`Downloads\pyr-source\push-pyr.ps1`, run by the
   scheduled task "Pyr backup to GitHub"; it only ever fast-forwards, and writes `push-pyr.log`).
   This session can't push to GitHub itself.
5. **Keys never in the repository, and never only in the workspace.** They live in
   `Downloads\pyr-keys` on Blue's PC (and the outputs folder). Use them by path; never print them.
   The pre-commit hook refuses anything that looks like a key.
6. **Anything else worth keeping** (tools, measurements, notes): in the repository, or zipped
   beside the bundle (`/home/claude/work/backup.sh` does the rebuild's notes and tools).

Starting again from nothing: clone `https://github.com/bluelight777/Pyr.git`, or
`git clone <bundle>` from the outputs folder or from Blue's PC, then step 1.

## The rebuild

`REBUILD.md`: what 0.13.1 to 0.13.28 did, what's rebuilt and checked against the real 0.13.28,
and the ground rules (talk to 0.13.28 as it did; voice and screen sharing identical to 0.13.28;
the echo test before anything that changes the echo canceller ships).
