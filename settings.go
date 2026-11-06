package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The 设置 window: on macOS in the app menu (BoundlessDeck → 设置…, ⌘,)
// as Finder and every Mac app has it; elsewhere at the end of 文件
// (设置…, Ctrl+,), where Windows apps keep theirs. It installs the global
// MCP into the agents on this computer (mcpglobal.go) and holds whether a
// folder opened gets project-level MCP configs.

// appSettings are the user's settings, kept in settings.json in the app's
// data folder.
type appSettings struct {
	// ProjectMCP writes .mcp.json, .cursor/mcp.json and .vscode/mcp.json
	// into each folder opened. Off by default: the folder may be a
	// repository, and the global install reaches the deck without them.
	ProjectMCP bool `json:"projectMCPConfigs"`
}

func (a *app) loadSettings() {
	if a.settingsFile == "" {
		return
	}
	data, err := os.ReadFile(a.settingsFile)
	if err != nil {
		return
	}
	var s appSettings
	if json.Unmarshal(data, &s) == nil {
		a.settings = s
	}
}

func (a *app) saveSettings() error {
	if a.settingsFile == "" {
		return nil
	}
	data, err := json.MarshalIndent(a.settings, "", "  ")
	if err != nil {
		return err
	}
	return writeFileAtomic(a.settingsFile, data)
}

// settingsState is what the 设置 window shows that is read off the UI
// thread: where each agent stands, the active deck, the open deck's
// project configs.
type settingsState struct {
	exe    string
	exeErr error
	agents []mcpAgent
	status map[string]mcpAgentStatus
	loaded bool
	// busy is the agent being installed or removed, "" for none.
	busy string
	// note says how the last action went, under the section it was in
	// (noteAt: "global" or "project"); noteErr that it failed.
	note    string
	noteAt  string
	noteErr bool
	// active is the deck the global bridge follows, "" for none.
	active string
	// projectRoot is the folder projectHasUs was read for.
	projectRoot  string
	projectHasUs bool
}

// showSettings opens the 设置 window, or brings it forward.
func (a *app) showSettings() {
	if a.setWin != nil && !a.setWin.IsDestroyed() {
		a.setWin.Show()
		a.setWin.Focus()
		a.refreshSettings()
		return
	}
	st := &settingsState{agents: userMCPAgents(), status: map[string]mcpAgentStatus{}}
	st.exe, st.exeErr = mcpInstallExe()
	a.setUI = st
	var w *mygo.Window
	w = mygo.NewWindow(mygo.WindowOptions{
		Title:             "设置",
		Width:             560,
		Height:            640,
		MinWidth:          460,
		MinHeight:         420,
		DisableMinimize:   true,
		DisableMaximize:   true,
		DisableFullScreen: true,
		BackgroundColor:   "light-dark(#f6f7f9, #0f1115)",
		StateKey:          "settings",
		Content: ui.View(func(c *ui.Context) {
			a.settingsView(c, func() { w.Close() })
		}),
	})
	a.setWin = w
	w.OnClosed(func() {
		if a.setWin == w {
			a.setWin, a.setUI = nil, nil
		}
	})
	a.refreshSettings()
}

// refreshSettings reads, off the UI thread, what the 设置 window shows.
func (a *app) refreshSettings() {
	st, w := a.setUI, a.setWin
	if st == nil || w == nil {
		return
	}
	agents, exe := st.agents, st.exe
	root := ""
	if a.deck != nil {
		root = a.deck.root
	}
	go func() {
		status := make(map[string]mcpAgentStatus, len(agents))
		for _, ag := range agents {
			status[ag.ID] = ag.status(exe)
		}
		active := ""
		if act, err := readMCPActive(); err == nil {
			active = act.Workspace
		}
		has := root != "" && projectFilesHaveUs(root)
		w.Update(func() {
			if a.setUI != st {
				return
			}
			st.status, st.loaded = status, true
			st.active = active
			st.projectRoot, st.projectHasUs = root, has
		})
	}()
}

// agentAction installs (or points at this app) or removes our server in
// one agent's config, off the UI thread.
func (a *app) agentAction(ag mcpAgent, install bool) {
	st, w := a.setUI, a.setWin
	if st == nil || w == nil || st.busy != "" {
		return
	}
	st.busy, st.note, st.noteAt = ag.ID, "", "global"
	exe := st.exe
	go func() {
		var err error
		if install {
			err = ag.install(exe)
		} else {
			err = ag.uninstall()
		}
		w.Update(func() {
			if a.setUI != st {
				return
			}
			st.busy = ""
			switch {
			case err != nil:
				st.note, st.noteErr = ag.Label+"："+err.Error(), true
			case install:
				st.note, st.noteErr = "已装到 "+ag.Label+"。重启它（或在 Claude Code 里用 /mcp 重连）后生效。", false
			default:
				st.note, st.noteErr = "已从 "+ag.Label+" 移除。", false
			}
			a.refreshSettings()
		})
	}()
}

// projectSwitched applies the 项目级配置 switch, which the view bound to
// a.settings.ProjectMCP: saved, and on the deck open at once. Turned off,
// the configs already in the folder stay until removed by hand.
func (a *app) projectSwitched() {
	st := a.setUI
	err := a.saveSettings()
	if a.deck != nil {
		if a.settings.ProjectMCP {
			err = errors.Join(err, a.deck.setProjectMCP(true))
		} else {
			a.deck.projectMCP = false
		}
	}
	if st != nil {
		st.noteAt = "project"
		if err != nil {
			st.note, st.noteErr = "项目级配置："+err.Error(), true
		} else {
			st.note = ""
		}
	}
	a.refreshSettings()
}

// removeProject takes our server out of the open deck's project configs.
func (a *app) removeProject() {
	st := a.setUI
	if a.deck == nil || st == nil {
		return
	}
	a.deck.projectMCP = false
	st.noteAt = "project"
	if err := removeProjectConfigs(a.deck.root); err != nil {
		st.note, st.noteErr = err.Error(), true
	} else {
		st.note, st.noteErr = "已从「"+filepath.Base(a.deck.root)+"」移除项目级 MCP 配置。", false
	}
	if ws, err := loadMCPWorkspace(a.deck.root); err == nil && a.deck.mcp != nil {
		a.deck.mcpProject = ws
	}
	a.refreshSettings()
}

func (a *app) settingsView(c *ui.Context, close func()) {
	t := c.Theme()
	st := a.setUI
	if c.Shortcut(0, ui.KeyEscape) && close != nil {
		close()
	}
	if st == nil {
		return
	}
	ui.Scroll(c).Fill().Background(t.Background).Children(func() {
		ui.Column(c).FillWidth().Padding(20, 24, 28, 24).Gap(26).Children(func() {
			settingsSection(c, t, "全局 MCP",
				"装进 agent 的用户级配置后，在任何文件夹里启动的 agent 都能操作 BoundlessDeck 当前打开的图版，项目里不用放任何配置。",
				func() { a.globalRows(c, t, st) })
			settingsSection(c, t, "当前图版", "", func() {
				settingsCard(c, t, func() {
					ui.Column(c).FillWidth().Padding(10, 14).Gap(3).Children(func() {
						if st.active == "" {
							ui.Text(c, "没有打开的图版").FontSize(13).TextColor(t.Text)
							ui.Text(c, "agent 此时调用工具，会收到「先在应用里打开一个文件夹」的提示。").
								FontSize(11).TextColor(t.TextMuted)
							return
						}
						ui.Text(c, filepath.Base(st.active)).FontSize(13).FontWeight(600).TextColor(t.Text)
						ui.Text(c, shortPath(st.active)).FontSize(11).TextColor(t.TextMuted).SingleLine()
					})
				})
			})
			settingsSection(c, t, "项目级配置", "", func() {
				a.projectRows(c, t, st)
				settingsNote(c, t, st, "project")
			})
		})
	})
}

// settingsNote shows how the last action of the section at went.
func settingsNote(c *ui.Context, t *ui.Theme, st *settingsState, at string) {
	if st.note == "" || st.noteAt != at {
		return
	}
	col := t.Success
	if st.noteErr {
		col = t.Danger
	}
	ui.Text(c, st.note).FontSize(12).TextColor(col)
}

// settingsSection is a titled group of the window.
func settingsSection(c *ui.Context, t *ui.Theme, title, about string, body func()) {
	ui.Column(c).FillWidth().Gap(8).Children(func() {
		ui.Text(c, title).FontSize(12).FontWeight(600).TextColor(t.TextMuted)
		if about != "" {
			ui.Text(c, about).FontSize(12).TextColor(t.TextMuted).LineHeight(1.45)
		}
		body()
	})
}

// settingsCard is the rounded panel rows sit in.
func settingsCard(c *ui.Context, t *ui.Theme, body func()) {
	ui.Column(c).FillWidth().Background(t.Surface).Border(1, t.Border).Radius(10).Clip().Children(body)
}

// globalRows lists the agents, each with its state and what to do.
func (a *app) globalRows(c *ui.Context, t *ui.Theme, st *settingsState) {
	settingsCard(c, t, func() {
		for i, ag := range st.agents {
			ag := ag
			s := st.status[ag.ID]
			row := ui.Row(c.Key("agent-"+ag.ID)).FillWidth().AlignItems(ui.Center).Gap(12).Padding(10, 14)
			if i > 0 {
				row = row.BorderWidth(1, 0, 0, 0).BorderColor(t.Border.Alpha(0.6))
			}
			row.Children(func() {
				ui.Column(c).Grow(1).Shrink(1).Gap(3).Children(func() {
					ui.Text(c, ag.Label).FontSize(13).TextColor(t.Text)
					text, col := agentStateText(t, s, st.loaded)
					ui.Text(c, text).FontSize(11).TextColor(col).MaxLines(2)
				})
				label, install, ok := agentButton(s)
				if !ok || !st.loaded {
					return
				}
				b := ui.Button(c.Key("agent-btn-"+ag.ID), label).FontSize(12).Padding(5, 12).Radius(7).
					Disabled(st.busy != "" || (install && st.exeErr != nil))
				if st.busy == ag.ID {
					b = b.Label("处理中…")
				}
				b.OnClick(func() { a.agentAction(ag, install) })
			})
		}
	})
	settingsNote(c, t, st, "global")
	if st.exeErr != nil {
		ui.Text(c, "暂时不能安装："+st.exeErr.Error()).FontSize(12).TextColor(t.Danger)
		return
	}
	ui.Text(c, "命令："+st.exe+" mcp-stdio").FontSize(11).TextColor(t.TextMuted).MaxLines(3)
}

// agentStateText says where an agent stands, and in what color.
func agentStateText(t *ui.Theme, s mcpAgentStatus, loaded bool) (string, ui.Color) {
	if !loaded {
		return "检查中…", t.TextMuted
	}
	switch s.State {
	case mcpAbsent:
		return "这台电脑上没有找到", t.TextMuted
	case mcpNotThere:
		return "未安装", t.TextMuted
	case mcpInstalled:
		return "已安装", t.Success
	case mcpElsewhere:
		return "已安装，但指向另一个 BoundlessDeck：" + s.Command, t.Warning
	case mcpBroken:
		msg := "配置文件读不了"
		if s.Err != nil {
			msg += "：" + s.Err.Error()
		}
		return msg, t.Danger
	}
	return "", t.TextMuted
}

// agentButton is the action an agent's row offers: its label, whether it
// installs, and whether there is one.
func agentButton(s mcpAgentStatus) (label string, install, ok bool) {
	switch s.State {
	case mcpNotThere:
		return "安装", true, true
	case mcpInstalled:
		return "卸载", false, true
	case mcpElsewhere:
		return "改为此应用", true, true
	}
	return "", false, false
}

// projectRows are the 项目级配置 switch and, for the deck open, the way to
// take its configs out.
func (a *app) projectRows(c *ui.Context, t *ui.Theme, st *settingsState) {
	settingsCard(c, t, func() {
		ui.Row(c).FillWidth().AlignItems(ui.Center).Gap(12).Padding(10, 14).Children(func() {
			ui.Column(c).Grow(1).Shrink(1).Gap(3).Children(func() {
				ui.Text(c, "打开文件夹时写入项目级 MCP 配置").FontSize(13).TextColor(t.Text)
				ui.Text(c, "在文件夹里写 .mcp.json、.cursor/mcp.json、.vscode/mcp.json，在里面启动的 agent 直接连这个图版。"+
					"文件夹是代码仓库时容易被误提交，所以默认关闭。").
					FontSize(11).TextColor(t.TextMuted).LineHeight(1.45)
			})
			ui.Switch(c.Key("project-mcp"), &a.settings.ProjectMCP).OnClick(a.projectSwitched)
		})
		d := a.deck
		if d == nil || st.projectRoot != d.root || !st.projectHasUs {
			return
		}
		ui.Row(c).FillWidth().AlignItems(ui.Center).Gap(12).Padding(10, 14).
			BorderWidth(1, 0, 0, 0).BorderColor(t.Border.Alpha(0.6)).Children(func() {
			ui.Column(c).Grow(1).Shrink(1).Gap(3).Children(func() {
				ui.Text(c, "「"+filepath.Base(d.root)+"」里有项目级配置").FontSize(13).TextColor(t.Text)
				ui.Text(c, "只删去 BoundlessDeck 这一项，文件里你自己的 server 保留。").FontSize(11).TextColor(t.TextMuted)
			})
			ui.Button(c.Key("project-remove"), "移除").FontSize(12).Padding(5, 12).Radius(7).OnClick(a.removeProject)
		})
	})
}
