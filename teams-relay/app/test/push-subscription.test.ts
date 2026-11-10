import { describe, expect, it } from "vitest";
import { deviceKindOf, subscriptionOf } from "@/shared/push-subscription";

const IPHONE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const IPAD_AS_MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const ANDROID = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

describe("the kind of device a browser subscribes from", () => {
  it("is mobile for an iPhone, an iPad reporting itself as a Mac with a touch screen, and Chrome on Android", () => {
    expect(deviceKindOf(IPHONE, 5)).toBe("mobile");
    expect(deviceKindOf(IPAD_AS_MAC, 5)).toBe("mobile");
    expect(deviceKindOf(ANDROID, 5)).toBe("mobile");
  });

  it("is desktop for Chrome on Windows and Safari on a Mac with no touch screen", () => {
    expect(deviceKindOf(WINDOWS, 0)).toBe("desktop");
    expect(deviceKindOf(IPAD_AS_MAC, 0)).toBe("desktop");
  });
});

describe("a subscription as stored", () => {
  const keys = { p256dh: "BNcRdreALRFXTkOOUHK1EtK2wtaz5Ry4YfYCA_0QTpQtUbVlUls0VJXg7A8u-Ts1XbjhazAkj7I99e8QcYP7DkM", auth: "tBHItJI5svbpez7KI4CCXg" };
  const sub = { endpoint: "https://push.example/abc", keys };

  it("keeps endpoint and keys, and says mobile when the browser said so", () => {
    expect(JSON.parse(subscriptionOf({ ...sub, device: "mobile" }).json)).toEqual({ ...sub, kind: "mobile" });
  });

  it("says desktop when the browser said nothing, or something else", () => {
    expect(JSON.parse(subscriptionOf(sub).json)).toEqual({ ...sub, kind: "desktop" });
    expect(JSON.parse(subscriptionOf({ ...sub, device: "toaster" }).json)).toEqual({ ...sub, kind: "desktop" });
  });
});
