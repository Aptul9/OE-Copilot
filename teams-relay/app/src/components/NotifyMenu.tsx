"use client";

import { BellOffIcon, ChevronDownIcon, MonitorIcon, MonitorSmartphoneIcon, SmartphoneIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { accName, type Account } from "@/lib/client";

export type NotifyChange = { notifyMobile?: boolean; notifyDesktop?: boolean };

// Where the notifications of an account go: tick Mobile, Desktop or both, none ticked is off
export function NotifyMenu({ account, onChange }: { account: Account; onChange: (to: NotifyChange) => void }) {
  const mobile = account.notifyMobile !== false;
  const desktop = account.notifyDesktop !== false;
  const Icon = mobile && desktop ? MonitorSmartphoneIcon : mobile ? SmartphoneIcon : desktop ? MonitorIcon : BellOffIcon;
  const word = mobile && desktop ? "Both" : mobile ? "Mobile" : desktop ? "Desktop" : "Off";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" className="h-10 w-full justify-between gap-1.5 px-2.5 font-normal sm:w-36 md:h-9" aria-label={`Notifications of ${accName(account)}`}>
          <span className="flex items-center gap-1.5">
            <Icon />
            {word}
          </span>
          <ChevronDownIcon className="text-muted-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-48">
        <DropdownMenuCheckboxItem checked={mobile} onCheckedChange={(on) => onChange({ notifyMobile: on === true })} onSelect={(e) => e.preventDefault()}>
          <SmartphoneIcon className="text-muted-foreground" />
          Mobile
        </DropdownMenuCheckboxItem>
        <DropdownMenuCheckboxItem checked={desktop} onCheckedChange={(on) => onChange({ notifyDesktop: on === true })} onSelect={(e) => e.preventDefault()}>
          <MonitorIcon className="text-muted-foreground" />
          Desktop
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
