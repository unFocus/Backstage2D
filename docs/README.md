# Planning docs

- [vision.md](vision.md): what Backstage2D is, and what it isn't
- [architecture.md](architecture.md): crate layout and core data model
- [timeline-and-display-list.md](timeline-and-display-list.md): how compositions, animations, and the evaluated scene behave
- [rendering.md](rendering.md): the renderer (what's implemented, what's next)
- [scripting.md](scripting.md): scripting language options
- [platforms.md](platforms.md): supported platforms (modern only, Wayland first)
- [dev-setup.md](dev-setup.md): building on the dev host (GTK via Homebrew)
- [testing.md](testing.md): automated tests: tiers, goldens, snapshots, `scripts/check.sh`
- [roadmap.md](roadmap.md): milestones
- [adr/](adr/): architecture decision records (one file per decision)
  - [0001: GUI framework](adr/0001-gui-framework.md)
  - [0002: Stage process isolation](adr/0002-stage-process-isolation.md)
  - [0003: Document model, scene model, and time](adr/0003-document-model.md)
  - [0004: Edit commands, the document log, and document authority](adr/0004-commands-and-document-authority.md)
