<script>
    import { onMount } from "svelte";
    import { readable } from "svelte/store";

    let socket; // Define socket here to make it accessible throughout the component
    let time = "";
    // Define realTimeData outside of the if block
    let realTimeData = readable([], (set) => {
        if (typeof WebSocket !== "undefined") {
            // This code will only run in the browser environment
            socket = new WebSocket("ws://10.0.1.177:5555");
            socket.addEventListener("open", () => {
                console.log("Opened");
                const message = JSON.stringify({ cmd: "GetData" });

                // Send the message
                socket.send(message);
            });

            // This will be triggered when the WebSocket messages arrive
            socket.addEventListener(
                "message",
                (/** @type {{ data: string; }} */ event) => {
                    const message = JSON.parse(event.data);
                    if (message.Data) {
                        console.log("Recv: Data: " + JSON.stringify(message));
                        time = time = new Date().toLocaleTimeString();
                        set(message.Data);
                    }
                }
            );
        }
    });

    // Function to format timestamp to a human-readable date
    /**
     * @param {string | number | Date} timestamp
     */
    function formatTimestamp(timestamp) {
        return new Date().toLocaleTimeString();
    }

    // Set up the periodic message sending after the WebSocket connection is established

    onMount(() => {
        console.log("on mount");
        if (socket) {
            async function fetchData() {
                const message = JSON.stringify({ cmd: "GetData" });
                console.log("periodic: " + message);
                // Send the message
                socket.send(message);
            }

            const interval = setInterval(fetchData, 3000);
            fetchData();

            return () => {
                console.log("onMount returned");
                clearInterval(interval);
            };
        }
    });
</script>

<main>
    <div id="data_section">
        <div>
            <h2>Realtime data</h2>
            <!-- {#each $realTimeData as data} -->
            <p>Localtime {time}</p>
            <p>SoC {$realTimeData.soc}%</p>
            <p>State {$realTimeData.state}</p>
            <p>Temperature {$realTimeData.temp}ºC</p>
            <p>Fan duty {$realTimeData.fan}%</p>
            <p>Amps {$realTimeData.amps}</p>
            <!-- {/each} -->
        </div>
    </div>
</main>
