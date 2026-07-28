import { useState, useEffect, useCallback } from "react";
import { fetchVersionInfo, upgradeController, fetchNodes, upgradeAgent } from "../api";
import type { VersionInfo, Node } from "../api";
import { useConfirm, useToast } from "./dialogs";

function UpdateBanner() {
  const [info, setInfo] = useState<VersionInfo | null>(null);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [upgrading, setUpgrading] = useState(false);
  const [picker, setPicker] = useState(false);
  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [agentBusy, setAgentBusy] = useState(false);
  const { confirmDialog, confirmElement } = useConfirm();
  const { showToast, toastElement } = useToast();

  const refresh = useCallback(() => {
    fetchVersionInfo().then(setInfo).catch(() => {});
    fetchNodes().then(setNodes).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, 60_000);
    return () => clearInterval(interval);
  }, [refresh]);

  const agentUpdate = info?.agent_update;
  // A node is upgradeable when it's connected and running an older agent than
  // the released one. Disconnected nodes can't be sent a command, so exclude them.
  const outdated = agentUpdate
    ? nodes.filter(
        (n) => n.live_status && n.agent_version && n.agent_version !== agentUpdate.version,
      )
    : [];

  const controllerUpdate = info?.controller_update;
  if (!controllerUpdate && outdated.length === 0) return null;

  const handleControllerUpgrade = async () => {
    if (!controllerUpdate) return;
    if (
      !(await confirmDialog({
        title: "Upgrade Controller",
        message: `Upgrade controller to v${controllerUpdate.version}? The controller will restart.`,
        confirmLabel: "Upgrade",
      }))
    )
      return;
    setUpgrading(true);
    try {
      await upgradeController();
      setTimeout(() => window.location.reload(), 5000);
    } catch (err) {
      showToast("error", `Upgrade failed: ${err}`);
      setUpgrading(false);
    }
  };

  const openPicker = () => {
    // Default every outdated node to selected.
    setSelected(Object.fromEntries(outdated.map((n) => [n.node_id, true])));
    setPicker(true);
  };

  const selectedIds = outdated.map((n) => n.node_id).filter((id) => selected[id]);

  const handleAgentUpgrade = async () => {
    if (selectedIds.length === 0) return;
    setAgentBusy(true);
    const results = await Promise.allSettled(selectedIds.map((id) => upgradeAgent(id)));
    const failed = results.filter((r) => r.status === "rejected").length;
    setAgentBusy(false);
    setPicker(false);
    if (failed > 0) {
      showToast(
        "error",
        `${selectedIds.length - failed} agent upgrade(s) started; ${failed} failed to dispatch.`,
      );
    } else {
      showToast("success", `${selectedIds.length} agent upgrade(s) started.`);
    }
    setTimeout(refresh, 3000);
  };

  return (
    <div className="bg-purple-900/40 border-b border-purple-500/30 w-full">
      <div className="max-w-7xl mx-auto px-6 py-3 flex flex-col gap-2">
        {controllerUpdate && (
          <div className="flex flex-col md:flex-row items-center justify-between gap-4">
            <span className="text-sm text-purple-200">
              Controller <strong>v{controllerUpdate.version}</strong> is available
              {controllerUpdate.release_notes && <> &mdash; {controllerUpdate.release_notes}</>}
            </span>
            <button
              className="px-4 py-1.5 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all whitespace-nowrap disabled:opacity-60"
              onClick={handleControllerUpgrade}
              disabled={upgrading}
            >
              {upgrading ? "Upgrading..." : "Upgrade Controller"}
            </button>
          </div>
        )}
        {agentUpdate && outdated.length > 0 && (
          <div className="flex flex-col md:flex-row items-center justify-between gap-4">
            <span className="text-sm text-purple-200">
              Agent <strong>v{agentUpdate.version}</strong> is available for{" "}
              {outdated.length} node{outdated.length === 1 ? "" : "s"}
            </span>
            <button
              className="px-4 py-1.5 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all whitespace-nowrap"
              onClick={openPicker}
            >
              Upgrade Agents…
            </button>
          </div>
        )}
      </div>

      {picker && agentUpdate && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
          onClick={() => !agentBusy && setPicker(false)}
        >
          <div
            className="w-full max-w-md bg-[#15131f] border border-white/10 rounded-xl shadow-xl p-6"
            onClick={(e) => e.stopPropagation()}
          >
            <h2 className="text-base font-semibold text-zinc-100 mb-1">
              Upgrade agents to v{agentUpdate.version}
            </h2>
            <p className="text-xs text-zinc-400 mb-4">
              Select the nodes to upgrade. Each agent downloads the new binary, verifies its
              checksum, and restarts.
            </p>
            <div className="flex flex-col gap-1 max-h-64 overflow-y-auto mb-4">
              {outdated.map((n) => (
                <label
                  key={n.node_id}
                  className="flex items-center gap-3 px-3 py-2 rounded hover:bg-white/[0.03] cursor-pointer"
                >
                  <input
                    type="checkbox"
                    checked={!!selected[n.node_id]}
                    onChange={(e) =>
                      setSelected((s) => ({ ...s, [n.node_id]: e.target.checked }))
                    }
                  />
                  <span className="text-sm text-zinc-200">{n.node_id}</span>
                  <span className="text-xs text-zinc-500 font-mono ml-auto">
                    v{n.agent_version} → v{agentUpdate.version}
                  </span>
                </label>
              ))}
            </div>
            <div className="flex items-center justify-between">
              <div className="flex gap-3 text-xs">
                <button
                  className="text-purple-400 hover:text-purple-300"
                  onClick={() =>
                    setSelected(Object.fromEntries(outdated.map((n) => [n.node_id, true])))
                  }
                >
                  Select all
                </button>
                <button
                  className="text-zinc-400 hover:text-zinc-300"
                  onClick={() => setSelected({})}
                >
                  Select none
                </button>
              </div>
              <div className="flex gap-2">
                <button
                  className="px-3 py-1.5 text-sm text-zinc-300 hover:text-white"
                  onClick={() => setPicker(false)}
                  disabled={agentBusy}
                >
                  Cancel
                </button>
                <button
                  className="px-4 py-1.5 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 disabled:opacity-60"
                  onClick={handleAgentUpgrade}
                  disabled={agentBusy || selectedIds.length === 0}
                >
                  {agentBusy
                    ? "Starting…"
                    : `Upgrade ${selectedIds.length} agent${selectedIds.length === 1 ? "" : "s"}`}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
      {confirmElement}
      {toastElement}
    </div>
  );
}

export default UpdateBanner;
