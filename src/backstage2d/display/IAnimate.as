package backstage2d.display 
{
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public interface IAnimate 
	{
		function play():void;
		function pause():void;
		function gotoFrame( frame:int ):void;
	}
}
