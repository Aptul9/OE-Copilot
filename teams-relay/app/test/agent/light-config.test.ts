import { describe, expect, it } from "vitest";
import { loadConfig } from "@/agent/config";

describe("light configuration", () => {
  it("allows native calls only with NATIVE_CALLS=1", () => {
    expect(loadConfig({ NATIVE_CALLS: "1" }).nativeCalls).toBe(true);
    expect(loadConfig({ NATIVE_CALLS: "0" }).nativeCalls).toBe(false);
    expect(loadConfig({}).nativeCalls).toBe(false);
  });

  it("has the calling host of the image unless CALLING_HOST says otherwise", () => {
    expect(loadConfig({}).callingHost).toBe("/app/calling-live-host.mjs");
    expect(loadConfig({ CALLING_HOST: "/tmp/host.mjs" }).callingHost).toBe("/tmp/host.mjs");
  });

  it("takes the audio socket from CALL_AUDIO_URL, else builds it from DOMAIN and the slot", () => {
    expect(loadConfig({ CALL_AUDIO_URL: "wss://x.example/socket", DOMAIN: "d.example", ACCOUNT: "3" }).callAudioUrl).toBe("wss://x.example/socket");
    expect(loadConfig({ DOMAIN: "d.example", ACCOUNT: "3" }).callAudioUrl).toBe("wss://d.example/api/call/audio/socket?a=3");
  });

  it("keeps the environment it was given, without the unset keys", () => {
    expect(loadConfig({ ACCOUNT: "2", TZ: "Europe/Rome", HOME: undefined }).env).toEqual({ ACCOUNT: "2", TZ: "Europe/Rome" });
  });
});
