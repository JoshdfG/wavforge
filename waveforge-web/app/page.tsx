"use client";
import { useRef, useState } from "react";
import { Bricolage_Grotesque } from "next/font/google";

const font = Bricolage_Grotesque({ subsets: ["latin"] });
const API = process.env.NEXT_PUBLIC_API_URL!;
const MAX_BYTES = 50 * 1024 * 1024;
const BARS = [
  30, 55, 40, 75, 50, 90, 65, 45, 80, 35, 60, 95, 70, 42, 58, 85, 48, 66, 38,
  72, 52, 88, 44, 62, 78, 36, 56, 92, 46, 68,
];

type Phase = "idle" | "uploading" | "converting" | "done" | "error";

export default function Home() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const [href, setHref] = useState("");
  const [over, setOver] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const busy = phase === "uploading" || phase === "converting";

  async function convert(file: File) {
    setName(file.name);
    setHref("");
    setError("");
    if (file.size > MAX_BYTES) {
      setError("That file is over 50 MB. Choose a smaller one.");
      return setPhase("error");
    }
    setPhase("uploading");
    try {
      const body = new FormData();
      body.append("file", file);
      const res = await fetch(`${API}/api/convert`, { method: "POST", body });
      const job = await res.json();
      if (!res.ok) throw new Error(job.error ?? "Upload failed.");

      setPhase("converting");
      for (;;) {
        await new Promise((r) => setTimeout(r, 700));
        const s = await (await fetch(`${API}/api/jobs/${job.id}`)).json();
        if (s.status === "failed")
          throw new Error(s.error ?? "Conversion failed.");
        if (s.status === "done") {
          setHref(`${API}/api/jobs/${job.id}/download`);
          return setPhase("done");
        }
      }
    } catch (e) {
      setError(
        e instanceof TypeError
          ? "Could not reach the server. Check your connection and try again."
          : e instanceof Error
            ? e.message
            : "Something went wrong.",
      );
      setPhase("error");
    }
  }

  const status = {
    idle: "Drop an m4a, mp4 or mp3 file",
    uploading: `Uploading ${name}…`,
    converting: `Converting ${name}…`,
    done: `${name} is ready`,
    error,
  }[phase];

  return (
    <main className={`${font.className} page`}>
      <h1>WavForge</h1>
      <p className="lede">
        Turn m4a, mp4 and mp3 audio into WAV. Files are deleted after 30
        minutes.
      </p>

      <button
        type="button"
        className={`drop ${over ? "over" : ""}`}
        data-phase={phase}
        disabled={busy}
        onClick={() => input.current?.click()}
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          const f = e.dataTransfer.files[0];
          if (f) convert(f);
        }}
      >
        <span className="wave" aria-hidden>
          {BARS.map((h, i) => (
            <i
              key={i}
              style={{ height: `${h}%`, animationDelay: `${i * 45}ms` }}
            />
          ))}
        </span>
        <span className="status" role="status">
          {status}
        </span>
        {phase === "idle" && (
          <span className="hint">or click to choose. Up to 50 MB.</span>
        )}
      </button>

      <input
        ref={input}
        type="file"
        hidden
        accept=".m4a,.mp4,.mp3,audio/*,video/mp4"
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) convert(f);
          e.target.value = "";
        }}
      />

      {phase === "done" && (
        <div className="actions">
          <a className="primary" href={href}>
            Download WAV
          </a>
          <button
            type="button"
            className="ghost"
            onClick={() => setPhase("idle")}
          >
            Convert another
          </button>
        </div>
      )}
      {phase === "error" && (
        <div className="actions">
          <button
            type="button"
            className="ghost"
            onClick={() => setPhase("idle")}
          >
            Try again
          </button>
        </div>
      )}
    </main>
  );
}
