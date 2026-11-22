import { usePathname } from "next/navigation";

/** The route, whatever URL shape served it: a static server without rewrites opens /impact.html, the binary
 *  serves /impact (FE-01). Comparing routes with the raw pathname would break hydration on the first. */
export const usePath = () => usePathname().replace(/(\/index)?\.html$|\/$/, "") || "/";
