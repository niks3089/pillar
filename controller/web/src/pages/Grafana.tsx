import { useEffect, useState } from "react";
import { fetchGrafanaSettings, saveGrafanaUrl, USE_MOCK } from "../api";

type Tab = "fleet-overview" | "node-detail";

function Grafana() {
  const [grafanaUrl, setGrafanaUrl] = useState("");
  const [urlInput, setUrlInput] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [activeTab, setActiveTab] = useState<Tab>("fleet-overview");
  const [showSettings, setShowSettings] = useState(false);

  useEffect(() => {
    fetchGrafanaSettings()
      .then((s) => {
        setGrafanaUrl(s.grafana_url);
        setUrlInput(s.grafana_url);
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, []);

  const handleSave = async () => {
    setSaving(true);
    setError("");
    try {
      const result = await saveGrafanaUrl(urlInput.trim());
      setGrafanaUrl(result.grafana_url);
      setShowSettings(false);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  if (loading)
    return (
      <div className="flex justify-center p-12 text-zinc-500">Loading...</div>
    );

  // Embedded view — iframe uses the controller's reverse proxy at /grafana/
  if (grafanaUrl && !showSettings) {
    const dashboardUid =
      activeTab === "fleet-overview"
        ? "pillar-fleet-overview"
        : "pillar-node-detail";
    const iframeSrc = `/grafana/d/${dashboardUid}?orgId=1&kiosk`;

    return (
      <div className="flex flex-col gap-4 w-full h-[calc(100vh-6rem)]">
        <div className="flex flex-wrap items-center justify-between gap-4 p-4 bg-[#15131f] border border-white/10 rounded-xl shadow-sm">
          <div className="flex gap-2">
            <button
              className={`px-4 py-2 text-sm font-medium rounded-md transition-colors ${activeTab === "fleet-overview" ? "bg-purple-500/20 text-purple-400" : "text-zinc-400 hover:text-zinc-200 hover:bg-white/5"}`}
              onClick={() => setActiveTab("fleet-overview")}
            >
              Fleet Overview
            </button>
            <button
              className={`px-4 py-2 text-sm font-medium rounded-md transition-colors ${activeTab === "node-detail" ? "bg-purple-500/20 text-purple-400" : "text-zinc-400 hover:text-zinc-200 hover:bg-white/5"}`}
              onClick={() => setActiveTab("node-detail")}
            >
              Node Detail
            </button>
          </div>
          <div className="flex gap-3 items-center">
            <a
              href={`/grafana/d/${dashboardUid}`}
              target="_blank"
              rel="noopener noreferrer"
              className="px-4 py-2 text-sm font-medium text-zinc-300 bg-white/5 hover:bg-white/10 rounded-md border border-white/10 shadow-sm transition-all"
            >
              Open in Grafana ↗
            </a>
            <button
              className="px-4 py-2 text-sm font-medium text-zinc-300 bg-white/5 hover:bg-white/10 rounded-md border border-white/10 shadow-sm transition-all"
              onClick={() => setShowSettings(true)}
            >
              Settings
            </button>
          </div>
        </div>
        <div className="flex-1 bg-[#15131f] border border-white/10 rounded-xl overflow-hidden shadow-sm flex flex-col relative">
          {USE_MOCK ? (
            <div className="flex items-center justify-center h-full text-zinc-500 bg-black/20">
              Unable to connect to the backend metrics service. Please ensure the controller is running and Grafana is configured.
            </div>
          ) : (
            <iframe
              src={iframeSrc}
              title={`Grafana - ${activeTab}`}
              className="w-full h-full border-none"
            />
          )}
        </div>
      </div>
    );
  }

  // Not configured / settings view
  return (
    <div className="flex flex-col max-w-2xl mx-auto mt-12 bg-[#15131f] border border-white/10 rounded-xl p-8 shadow-sm">
      <h2 className="text-xl font-semibold text-zinc-100 mb-2">
        {showSettings ? "Grafana Settings" : "Grafana"}
      </h2>
      <p className="text-sm text-zinc-400 mb-6">
        {grafanaUrl
          ? "Update the local Grafana URL that the controller proxies to."
          : "Enter the local Grafana URL (e.g. http://localhost:3000). The controller reverse-proxies it so dashboards are accessible remotely."}
      </p>
      <div className="flex flex-col gap-1.5">
        <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">
          Local Grafana URL
        </label>
        <div className="flex gap-3">
          <input
            type="text"
            value={urlInput}
            onChange={(e) => setUrlInput(e.target.value)}
            placeholder="http://localhost:3000"
            className="flex-1 px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600"
          />
          <button
            className="px-5 py-2 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all disabled:opacity-50"
            onClick={handleSave}
            disabled={saving}
          >
            {saving ? "Saving..." : "Save"}
          </button>
          {showSettings && (
            <button
              className="px-4 py-2 text-sm font-medium text-zinc-300 bg-white/5 hover:bg-white/10 rounded-md border border-white/10 shadow-sm transition-all"
              onClick={() => setShowSettings(false)}
            >
              Cancel
            </button>
          )}
        </div>
        {error && <p className="text-sm text-red-400 mt-1">{error}</p>}
      </div>
    </div>
  );
}

export default Grafana;
