# UI polish evidence

Baseline: GitHub main at `fa07b09a602656055ec52aa9a09c64bbbb02fbc7`. Captured headlessly with isolated sample data; no native windows. The same 13 states in both themes are retained before and after. Layout-menu images show the new combined control; the baseline workspace shows its three former buttons.

Changes: consistent SVG action geometry and explicit 8px folder disclosure/icon/label gaps; sidebar search/add/recovery/duplicate actions; balanced 18px branding insets; explicit workspace views and agent visibility in Layout; expandable sidebar trust details; hidden editor actions during expanded chat; unclipped conversation context; Favorites spacing; version metadata separators; Escape dismissal for file search.

## Visual comparisons

### populated (light)

| Before | After |
| --- | --- |
| ![Before populated light](before/populated-light.png) | ![After populated light](after/populated-light.png) |

### populated (dark)

| Before | After |
| --- | --- |
| ![Before populated dark](before/populated-dark.png) | ![After populated dark](after/populated-dark.png) |

### grid (light)

| Before | After |
| --- | --- |
| ![Before grid light](before/grid-light.png) | ![After grid light](after/grid-light.png) |

### grid (dark)

| Before | After |
| --- | --- |
| ![Before grid dark](before/grid-dark.png) | ![After grid dark](after/grid-dark.png) |

### workspace (light)

| Before | After |
| --- | --- |
| ![Before workspace light](before/workspace-light.png) | ![After workspace light](after/workspace-light.png) |

### workspace (dark)

| Before | After |
| --- | --- |
| ![Before workspace dark](before/workspace-dark.png) | ![After workspace dark](after/workspace-dark.png) |

### dialog (light)

| Before | After |
| --- | --- |
| ![Before dialog light](before/dialog-light.png) | ![After dialog light](after/dialog-light.png) |

### dialog (dark)

| Before | After |
| --- | --- |
| ![Before dialog dark](before/dialog-dark.png) | ![After dialog dark](after/dialog-dark.png) |

### tools (light)

| Before | After |
| --- | --- |
| ![Before tools light](before/tools-light.png) | ![After tools light](after/tools-light.png) |

### tools (dark)

| Before | After |
| --- | --- |
| ![Before tools dark](before/tools-dark.png) | ![After tools dark](after/tools-dark.png) |

### comments (light)

| Before | After |
| --- | --- |
| ![Before comments light](before/comments-light.png) | ![After comments light](after/comments-light.png) |

### comments (dark)

| Before | After |
| --- | --- |
| ![Before comments dark](before/comments-dark.png) | ![After comments dark](after/comments-dark.png) |

### versions (light)

| Before | After |
| --- | --- |
| ![Before versions light](before/versions-light.png) | ![After versions light](after/versions-light.png) |

### versions (dark)

| Before | After |
| --- | --- |
| ![Before versions dark](before/versions-dark.png) | ![After versions dark](after/versions-dark.png) |

### models (light)

| Before | After |
| --- | --- |
| ![Before models light](before/models-light.png) | ![After models light](after/models-light.png) |

### models (dark)

| Before | After |
| --- | --- |
| ![Before models dark](before/models-dark.png) | ![After models dark](after/models-dark.png) |

### compact (light)

| Before | After |
| --- | --- |
| ![Before compact light](before/compact-light.png) | ![After compact light](after/compact-light.png) |

### compact (dark)

| Before | After |
| --- | --- |
| ![Before compact dark](before/compact-dark.png) | ![After compact dark](after/compact-dark.png) |

### compact-preview (light)

| Before | After |
| --- | --- |
| ![Before compact-preview light](before/compact-preview-light.png) | ![After compact-preview light](after/compact-preview-light.png) |

### compact-preview (dark)

| Before | After |
| --- | --- |
| ![Before compact-preview dark](before/compact-preview-dark.png) | ![After compact-preview dark](after/compact-preview-dark.png) |

### short (light)

| Before | After |
| --- | --- |
| ![Before short light](before/short-light.png) | ![After short light](after/short-light.png) |

### short (dark)

| Before | After |
| --- | --- |
| ![Before short dark](before/short-dark.png) | ![After short dark](after/short-dark.png) |

### conversation (light)

| Before | After |
| --- | --- |
| ![Before conversation light](before/conversation-light.png) | ![After conversation light](after/conversation-light.png) |

### conversation (dark)

| Before | After |
| --- | --- |
| ![Before conversation dark](before/conversation-dark.png) | ![After conversation dark](after/conversation-dark.png) |

### expanded-chat (light)

| Before | After |
| --- | --- |
| ![Before expanded-chat light](before/expanded-chat-light.png) | ![After expanded-chat light](after/expanded-chat-light.png) |

### expanded-chat (dark)

| Before | After |
| --- | --- |
| ![Before expanded-chat dark](before/expanded-chat-dark.png) | ![After expanded-chat dark](after/expanded-chat-dark.png) |

## Combined Layout menu

![Layout light](after/layout-light.png)

![Layout dark](after/layout-dark.png)

## Verification

- `npm run build`: TypeScript and asset build passed.
- Headless UI suite: seven tests passed, including layout transitions, sidebar geometry, trust disclosure, search dismissal, keyboard behavior, dialogs, composer, comments and model picker.
- Capture script: 26 screenshots with 13 identical light/dark fixture-state pairs, plus two Layout-menu and two minimum-width branding captures.
- Hidden Electron preferences persistence check passed. A separate live-find check timed out waiting for its externally created `other.tex` file; that check is not counted among the passing headless suite.

## Independent review

GPT-6.1 Sol at high reasoning reviewed the visuals and interactions independently. The first pass found icon, scrolling, spacing and keyboard issues; the second pass found three Layout transition/dismissal issues. Those findings were addressed with regression coverage. The final whole-app review awarded 10/10 in all twelve sections after independent keyboard checks and inspection of 22 representative after images. Scores mean no remaining concrete actionable defect found within the reviewed polish scope. This is not a claim of exhaustive native-window or screen-reader verification.

| Section | Final score |
| --- | --- |
| Icons and alignment | 10/10 |
| Sidebar | 10/10 |
| Workspace / Layout | 10/10 |
| Editor / tabs | 10/10 |
| Assistant / composer | 10/10 |
| Model picker | 10/10 |
| PDF | 10/10 |
| Tools / comments / versions | 10/10 |
| Dialogs / recovery | 10/10 |
| Themes | 10/10 |
| Compact layout | 10/10 |
| Keyboard / accessibility | 10/10 |

## Minimum-width branding

The logo stays 28 x 28px at the 150px sidebar minimum; only the title truncates. Geometry and collapsed-sidebar checks pass.

![Narrow branding light](after/branding-narrow-light.png)

![Narrow branding dark](after/branding-narrow-dark.png)

The supplemental GPT-6.1 Sol high branding re-review rated spacing/alignment, sidebar consistency, light/dark, and compact/collapsed behavior 10/10. It independently measured both themes at normal and minimum sidebar widths and found no actionable defect.

Branding starts at the actual window corner with 18px top and left insets, above the breadcrumb baseline. The Projects / project-name breadcrumb remains a single horizontal group below the 28px native window-control strip. Branding controls are excluded from the draggable titlebar so Home and the sidebar toggle remain clickable.

The model-name button omits its decorative dropdown arrow; its picker and accessible expanded state remain available. Header regression checks use actual viewport coordinates and verify the breadcrumb components share one lower baseline.

The window-corner follow-up review rated branding insets, breadcrumb hierarchy/alignment, light/dark, compact/collapsed, and clickability/keyboard 10/10. Independent headless checks measured logo x18/y18 and breadcrumb center y56 versus logo center y32. Native Electron dragging was not exercised. All seven headless UI tests passed after this update.
